//! NEAR Intents top-ups through 1Click: the agent moves the account's funds
//! from one chain to the account's own address on another.
//!
//! 1Click answers a quote with a deposit address and signs the quote with its
//! ed25519 key. The agent hands the extension that signed quote (the exact
//! JSON 1Click signed) together with the deposit transfer. The extension:
//! - checks 1Click's signature;
//! - requires the swap to pay the account's own address on the destination
//!   chain and to refund the account's own address on the origin chain;
//! - requires an exact-input swap, at most 3% slippage, and an output worth at
//!   least 95% of the input by 1Click's own USD prices (so no hidden app fee
//!   can take the value);
//! - lets the deposit address be paid in this one transfer, for at most the
//!   quoted amount of the quoted asset, counted against the usual caps.
//!
//! The signed message, from 1Click's SDK (defuse-protocol/one-click-sdk-
//! typescript, `src/quote-signature.ts`): the UTF-8 bytes of
//! base58(sha256(json-stable-stringify({...request, ...quote, timestamp}))),
//! verified against `ONE_CLICK_KEY`, the key that file names. Checked against
//! live mainnet quotes on 13 November 2026 (tests).

use near_sdk::{env, serde_json::Value, AccountId};

use crate::{evm, rules::Grant, sol};

/// 1Click's quote-signing key (`ONE_CLICK_MANAGER_PUB_KEY`), base58.
pub const ONE_CLICK_KEY: &str = "reYaWhvwu8Jzo3WUM3zhn6VrhuMEF4eADL17qtRVifc";
pub const MAX_SLIPPAGE_BPS: u64 = 300;
/// The output must be worth at least this share of the input (per mille).
pub const MIN_VALUE_PERMILLE: u128 = 950;
/// A quote is used within this long of being made.
pub const MAX_QUOTE_AGE_MS: u64 = 30 * 60_000;
/// What a Solana deposit may also pay in fees, lamports.
pub const SOL_FEE_ALLOWANCE: u128 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Near,
    Evm(u64),
    Sol,
    Tron,
    Xrp,
}

/// The assets a top-up may move: (1Click asset ID, chain, token).
/// From 1Click's `/v0/tokens` (13 November 2026), whose `contractAddress`
/// gives each token. A token of `None` is the chain's own coin.
const ASSETS: &[(&str, Family, Option<&str>)] = &[
    ("nep141:eth.omft.near", Family::Evm(1), None),
    ("nep141:eth-0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48.omft.near", Family::Evm(1), Some("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48")),
    ("nep141:eth-0xdac17f958d2ee523a2206206994597c13d831ec7.omft.near", Family::Evm(1), Some("0xdac17f958d2ee523a2206206994597c13d831ec7")),
    ("nep141:base.omft.near", Family::Evm(8453), None),
    ("nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near", Family::Evm(8453), Some("0x833589fcd6edb6e08f4c7c32d4f71b54bda02913")),
    ("nep141:arb.omft.near", Family::Evm(42161), None),
    ("nep141:arb-0xaf88d065e77c8cc2239327c5edb3a432268e5831.omft.near", Family::Evm(42161), Some("0xaf88d065e77c8cc2239327c5edb3a432268e5831")),
    ("nep141:bnb.omdep.near", Family::Evm(56), None),
    ("nep141:bnb-0x8ac76a51cc950d9822d68b83fe1ad97b32cd580d.omdep.near", Family::Evm(56), Some("0x8ac76a51cc950d9822d68b83fe1ad97b32cd580d")),
    ("nep141:bnb-0x55d398326f99059ff775485246999027b3197955.omdep.near", Family::Evm(56), Some("0x55d398326f99059ff775485246999027b3197955")),
    ("nep141:pol.omdep.near", Family::Evm(137), None),
    ("nep141:pol-0x3c499c542cef5e3811e1192ce70d8cc03d5c3359.omdep.near", Family::Evm(137), Some("0x3c499c542cef5e3811e1192ce70d8cc03d5c3359")),
    ("nep141:pol-0xc2132d05d31c914a87c6611c10748aeb04b58e8f.omdep.near", Family::Evm(137), Some("0xc2132d05d31c914a87c6611c10748aeb04b58e8f")),
    ("nep141:avax.omdep.near", Family::Evm(43114), None),
    ("nep141:avax-0xb97ef9ef8734c71904d8002f8b6bc66dd9c48a6e.omdep.near", Family::Evm(43114), Some("0xb97ef9ef8734c71904d8002f8b6bc66dd9c48a6e")),
    ("nep141:avax-0x9702230a8ea53601f5cd2dc00fdbc13d4df4a8c7.omdep.near", Family::Evm(43114), Some("0x9702230a8ea53601f5cd2dc00fdbc13d4df4a8c7")),
    ("nep141:sol.omft.near", Family::Sol, None),
    ("nep141:sol-5ce3bf3a31af18be40ba30f721101b4341690186.omft.near", Family::Sol, Some("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")),
    ("nep141:sol-c800a4bd850783ccb82c2b2c7e84175443606352.omft.near", Family::Sol, Some("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB")),
    // Destinations only: the extension can't yet pay a deposit address there.
    ("nep141:wrap.near", Family::Near, Some("wrap.near")),
    ("nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1", Family::Near, Some("17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1")),
    ("nep141:usdt.tether-token.near", Family::Near, Some("usdt.tether-token.near")),
    ("nep141:tron.omft.near", Family::Tron, None),
    ("nep141:tron-d28a265909efecdcee7c5028585214ea0b96f015.omft.near", Family::Tron, Some("TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t")),
    ("nep141:xrp.omft.near", Family::Xrp, None),
];

pub fn asset(id: &str) -> Option<(Family, Option<&'static str>)> {
    ASSETS.iter().find(|(a, _, _)| *a == id).map(|(_, f, t)| (*f, *t))
}

/// What a checked quote lets the deposit transfer do.
#[derive(Debug, PartialEq, Eq)]
pub struct Deposit {
    pub origin: Family,
    /// The deposit address, as the origin chain's rules write it.
    pub address: String,
    /// The cap asset the transfer spends (`evm:<chain>`, `evm:<chain>:<token>`,
    /// `sol`, `spl:<mint>`).
    pub spend: String,
    pub amount_in: u128,
}

/// The message 1Click signs over a quote's canonical JSON.
pub fn signed_message(quote: &str) -> String {
    bs58::encode(env::sha256_array(quote.as_bytes())).into_string()
}

fn s<'a>(v: &'a Value, k: &str) -> Result<&'a str, &'static str> {
    v[k].as_str().ok_or("quote is missing a field")
}

fn amount(v: &Value, k: &str) -> Result<u128, &'static str> {
    s(v, k)?.parse().map_err(|_| "bad amount in quote")
}

/// A USD amount ("7.831000000000") in millionths of a dollar.
pub fn usd_micros(x: &str) -> Result<u128, &'static str> {
    let (w, f) = x.split_once('.').unwrap_or((x, ""));
    if w.is_empty() || !w.bytes().all(|b| b.is_ascii_digit()) || !f.bytes().all(|b| b.is_ascii_digit()) || w.len() > 20 {
        return Err("bad USD amount in quote");
    }
    let f6: String = f.chars().chain(core::iter::repeat('0')).take(6).collect();
    Ok(w.parse::<u128>().map_err(|_| "bad USD amount in quote")? * 1_000_000 + f6.parse::<u128>().map_err(|_| "bad USD amount in quote")?)
}

/// "2026-11-13T04:05:12.514Z" as Unix milliseconds.
pub fn iso_ms(t: &str) -> Result<u64, &'static str> {
    let e = "bad time in quote";
    let b = t.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' || *b.last().unwrap() != b'Z' {
        return Err(e);
    }
    let n = |r: core::ops::Range<usize>| -> Result<i64, &'static str> { t.get(r).and_then(|x| x.parse::<i64>().ok()).ok_or(e) };
    let (y, m, d, hh, mm, ss) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    let ms = match &t[19..t.len() - 1] {
        "" => 0,
        f if f.len() == 4 && f.starts_with('.') => f[1..].parse::<i64>().map_err(|_| e)?,
        _ => return Err(e),
    };
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 60 {
        return Err(e);
    }
    // days from civil (H. Hinnant)
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(((days * 86_400 + hh * 3600 + mm * 60 + ss) * 1000) + ms).map_err(|_| e)
}

/// The account's own address on a chain, as the grant names it.
fn own(g: &Grant, f: Family, wallet: &AccountId) -> Result<String, &'static str> {
    Ok(match f {
        Family::Near => wallet.to_string(),
        Family::Evm(_) => g.evm_self.clone().ok_or("this grant doesn't name the account's EVM address")?.to_lowercase(),
        Family::Sol => g.sol.as_ref().ok_or("this grant doesn't name the account's Solana address")?.self_address.clone(),
        Family::Tron => g.tron.as_ref().ok_or("this grant doesn't name the account's TRON address")?.self_address.clone(),
        Family::Xrp => g.xrpl.as_ref().ok_or("this grant doesn't name the account's XRP address")?.self_address.clone(),
    })
}

fn same(f: Family, a: &str, b: &str) -> bool {
    match f {
        Family::Evm(_) => a.to_lowercase() == b.to_lowercase(),
        _ => a == b,
    }
}

/// Checks a signed 1Click quote; returns what the deposit transfer may do.
pub fn check(g: &Grant, quote: &str, signature: &str, now_ms: u64, wallet: &AccountId) -> Result<Deposit, &'static str> {
    let sig: [u8; 64] = bs58::decode(signature.strip_prefix("ed25519:").unwrap_or(signature))
        .into_vec()
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or("bad quote signature")?;
    let key: [u8; 32] = bs58::decode(ONE_CLICK_KEY).into_vec().ok().and_then(|v| v.try_into().ok()).ok_or("bad key")?;
    if !env::ed25519_verify(&sig, signed_message(quote).as_bytes(), &key) {
        return Err("quote not signed by 1Click");
    }
    let q: Value = near_sdk::serde_json::from_str(quote).map_err(|_| "quote is not JSON")?;
    if q["dry"] != Value::Bool(false) {
        return Err("a dry quote has no deposit address");
    }
    if s(&q, "swapType")? != "EXACT_INPUT" || s(&q, "depositType")? != "ORIGIN_CHAIN" || s(&q, "refundType")? != "ORIGIN_CHAIN" || s(&q, "recipientType")? != "DESTINATION_CHAIN" {
        return Err("only exact-input swaps from and to the chains themselves");
    }
    for k in ["depositMemo", "customRecipientMsg", "virtualChainRecipient", "virtualChainRefundRecipient"] {
        if !q[k].is_null() {
            return Err("quote has a field the extension doesn't allow");
        }
    }
    if q["slippageTolerance"].as_u64().ok_or("quote is missing a field")? > MAX_SLIPPAGE_BPS {
        return Err("slippage over 3%");
    }
    let (origin, token) = asset(s(&q, "originAsset")?).ok_or("origin asset not supported")?;
    let (dest, _) = asset(s(&q, "destinationAsset")?).ok_or("destination asset not supported")?;
    if s(&q, "originAsset")? == s(&q, "destinationAsset")? {
        return Err("same asset on both sides");
    }
    if !same(dest, s(&q, "recipient")?, &own(g, dest, wallet)?) {
        return Err("a top-up pays only the account itself");
    }
    if !same(origin, s(&q, "refundTo")?, &own(g, origin, wallet)?) {
        return Err("refunds must go to the account itself");
    }
    let (inp, out) = (usd_micros(s(&q, "amountInUsd")?)?, usd_micros(s(&q, "amountOutUsd")?)?);
    if out.checked_mul(1000).ok_or("overflow")? < inp.checked_mul(MIN_VALUE_PERMILLE).ok_or("overflow")? {
        return Err("the quote loses more than 5% of the value");
    }
    let made = iso_ms(s(&q, "timestamp")?)?;
    if made > now_ms.saturating_add(60_000) || now_ms > made.saturating_add(MAX_QUOTE_AGE_MS) {
        return Err("quote too old");
    }
    if iso_ms(s(&q, "deadline")?)? < now_ms.saturating_add(10 * 60_000) {
        return Err("the deposit address is about to expire");
    }
    let address = s(&q, "depositAddress")?;
    let (address, spend) = match (origin, token) {
        (Family::Evm(c), t) => {
            let a = evm::fmt_address(&evm::parse_address(address)?);
            (a, match t { None => format!("evm:{c}"), Some(t) => format!("evm:{c}:{t}") })
        }
        (Family::Sol, t) => {
            let a = sol::fmt(&sol::b58(address)?);
            (a, match t { None => "sol".to_string(), Some(m) => format!("spl:{m}") })
        }
        _ => return Err("the extension can't pay a deposit address on that chain yet"),
    };
    Ok(Deposit { origin, address, spend, amount_in: amount(&q, "amountIn")? })
}
