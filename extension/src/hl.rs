//! Hyperliquid L1 actions (orders, cancels, leverage), signed for the
//! account's EVM address through Chain Signatures.
//!
//! The agent never sends bytes to sign. It sends the order's fields; the
//! extension encodes the action itself (MessagePack, as Hyperliquid's SDK
//! does), hashes it with the nonce into the "phantom agent" connection ID,
//! and builds the EIP-712 digest Hyperliquid verifies:
//! `Agent(string source, bytes32 connectionId)` in the `Exchange` domain
//! (chain ID 1337). `source` is fixed to `b`, Hyperliquid testnet, so a
//! signature from this extension is valid only for that exchange and that
//! network, and can't be an EVM transaction (whose preimage starts 0x02).

use near_sdk::env;

/// Hyperliquid testnet. A mainnet build would use "a".
pub const SOURCE: &str = "b";

/// The order types Hyperliquid accepts for limit orders.
pub const TIFS: [&str; 3] = ["Gtc", "Ioc", "Alo"];

// ---- MessagePack, only the forms these actions use ----

fn mp_str(out: &mut Vec<u8>, s: &str) {
    let n = s.len();
    if n < 32 {
        out.push(0xa0 | n as u8);
    } else {
        out.push(0xd9);
        out.push(n as u8);
    }
    out.extend(s.as_bytes());
}

fn mp_uint(out: &mut Vec<u8>, v: u64) {
    match v {
        0..=0x7f => out.push(v as u8),
        0x80..=0xff => out.extend([0xcc, v as u8]),
        0x100..=0xffff => {
            out.push(0xcd);
            out.extend((v as u16).to_be_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(0xce);
            out.extend((v as u32).to_be_bytes());
        }
        _ => {
            out.push(0xcf);
            out.extend(v.to_be_bytes());
        }
    }
}

fn mp_bool(out: &mut Vec<u8>, b: bool) {
    out.push(if b { 0xc3 } else { 0xc2 });
}

fn mp_map(out: &mut Vec<u8>, n: u8) {
    out.push(0x80 | n);
}

/// A decimal as Hyperliquid's SDK writes it (`float_to_wire`): digits, at
/// most one point, no leading zeros, no trailing zeros after the point, at
/// most 8 decimals, not zero. Returns (digits as an integer, decimals).
pub fn parse_decimal(s: &str) -> Result<(u128, u32), &'static str> {
    const BAD: &str = "price and size must be plain decimals, e.g. 2143.5";
    if s.is_empty() || s.len() > 24 {
        return Err(BAD);
    }
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i, f),
        None => (s, ""),
    };
    if int.is_empty() || !int.bytes().all(|b| b.is_ascii_digit()) || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return Err(BAD);
    }
    if (int.len() > 1 && int.starts_with('0')) || (s.contains('.') && (frac.is_empty() || frac.ends_with('0'))) || frac.len() > 8 {
        return Err(BAD);
    }
    let v: u128 = format!("{int}{frac}").parse().map_err(|_| BAD)?;
    if v == 0 {
        return Err("price and size must be above zero");
    }
    Ok((v, frac.len() as u32))
}

/// An order's value in micro-USD (6 decimals), price × size, rounded up.
pub fn notional_micro_usd(price: &str, size: &str) -> Result<u128, &'static str> {
    let (p, pd) = parse_decimal(price)?;
    let (s, sd) = parse_decimal(size)?;
    let prod = p.checked_mul(s).ok_or("order too large")?;
    let scale = pd + sd;
    Ok(if scale <= 6 {
        prod.checked_mul(10u128.pow(6 - scale)).ok_or("order too large")?
    } else {
        prod.div_ceil(10u128.pow(scale - 6))
    })
}

/// `{"type":"order","orders":[{a,b,p,s,r,t:{limit:{tif}}}],"grouping":"na"}`.
pub fn order_action(asset: u32, is_buy: bool, price: &str, size: &str, reduce_only: bool, tif: &str) -> Vec<u8> {
    let mut o = vec![];
    mp_map(&mut o, 3);
    mp_str(&mut o, "type");
    mp_str(&mut o, "order");
    mp_str(&mut o, "orders");
    o.push(0x91); // array of one
    mp_map(&mut o, 6);
    mp_str(&mut o, "a");
    mp_uint(&mut o, asset as u64);
    mp_str(&mut o, "b");
    mp_bool(&mut o, is_buy);
    mp_str(&mut o, "p");
    mp_str(&mut o, price);
    mp_str(&mut o, "s");
    mp_str(&mut o, size);
    mp_str(&mut o, "r");
    mp_bool(&mut o, reduce_only);
    mp_str(&mut o, "t");
    mp_map(&mut o, 1);
    mp_str(&mut o, "limit");
    mp_map(&mut o, 1);
    mp_str(&mut o, "tif");
    mp_str(&mut o, tif);
    mp_str(&mut o, "grouping");
    mp_str(&mut o, "na");
    o
}

/// `{"type":"cancel","cancels":[{a,o}]}`.
pub fn cancel_action(asset: u32, oid: u64) -> Vec<u8> {
    let mut o = vec![];
    mp_map(&mut o, 2);
    mp_str(&mut o, "type");
    mp_str(&mut o, "cancel");
    mp_str(&mut o, "cancels");
    o.push(0x91);
    mp_map(&mut o, 2);
    mp_str(&mut o, "a");
    mp_uint(&mut o, asset as u64);
    mp_str(&mut o, "o");
    mp_uint(&mut o, oid);
    o
}

/// `{"type":"updateLeverage","asset":a,"isCross":c,"leverage":l}`.
pub fn leverage_action(asset: u32, is_cross: bool, leverage: u32) -> Vec<u8> {
    let mut o = vec![];
    mp_map(&mut o, 4);
    mp_str(&mut o, "type");
    mp_str(&mut o, "updateLeverage");
    mp_str(&mut o, "asset");
    mp_uint(&mut o, asset as u64);
    mp_str(&mut o, "isCross");
    mp_bool(&mut o, is_cross);
    mp_str(&mut o, "leverage");
    mp_uint(&mut o, leverage as u64);
    o
}

fn keccak(b: &[u8]) -> [u8; 32] {
    env::keccak256_array(b)
}

/// The EIP-712 digest Hyperliquid checks for an L1 action from the account
/// itself (no vault, no expiry).
pub fn signing_hash(action_msgpack: &[u8], nonce: u64) -> [u8; 32] {
    let mut data = action_msgpack.to_vec();
    data.extend(nonce.to_be_bytes());
    data.push(0); // no vault
    let connection_id = keccak(&data);

    let mut domain = keccak(b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)").to_vec();
    domain.extend(keccak(b"Exchange"));
    domain.extend(keccak(b"1"));
    let mut chain_id = [0u8; 32];
    chain_id[30..].copy_from_slice(&1337u16.to_be_bytes());
    domain.extend(chain_id);
    domain.extend([0u8; 32]); // verifyingContract: the zero address
    let domain_separator = keccak(&domain);

    let mut agent = keccak(b"Agent(string source,bytes32 connectionId)").to_vec();
    agent.extend(keccak(SOURCE.as_bytes()));
    agent.extend(connection_id);
    let struct_hash = keccak(&agent);

    let mut msg = vec![0x19, 0x01];
    msg.extend(domain_separator);
    msg.extend(struct_hash);
    keccak(&msg)
}
