//! Polymarket orders (Polygon): the agent asks for a CLOB order to be signed;
//! the extension checks it and builds the EIP-712 hash itself, which the
//! account's EVM key signs off-chain. Polymarket's operator matches and
//! settles orders on its CTF exchange.
//!
//! From Polymarket/ctf-exchange: `Order` and ORDER_TYPEHASH
//! (libraries/OrderStructs.sol), the domain `Hashing("Polymarket CTF
//! Exchange", "1")` (CTFExchange.sol). On a fork of Polygon the exchanges'
//! own `hashOrder` gives the same hash (tests).
//!
//! An order the extension signs:
//! - on an exchange the grant lists as a venue (chain 137), one of the two
//!   Polymarket exchanges below, whose collateral is USDC.e;
//! - the account is maker and signer, signing as an EOA (type 0), open to
//!   any taker;
//! - fee at most 2%, and it expires within 30 days;
//! - a buy counts its USDC.e (`makerAmount`) against `evm:137:<USDC.e>`; a
//!   sell counts its shares against `polymarket:sell` (the grant's limit on
//!   what the agent may sell a day, at any price).

use near_sdk::{env, json_types::U128, near};

use crate::evm;
use crate::rules::Grant;

const ORDER_TYPEHASH: &[u8] = b"Order(uint256 salt,address maker,address signer,address taker,uint256 tokenId,uint256 makerAmount,uint256 takerAmount,uint256 expiration,uint256 nonce,uint256 feeRateBps,uint8 side,uint8 signatureType)";
const DOMAIN_TYPEHASH: &[u8] = b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";
pub const POLYGON: u64 = 137;
/// The CTF exchange and the neg-risk CTF exchange; `getCollateral()` of both
/// is USDC.e (checked on a fork of Polygon).
pub const EXCHANGES: [&str; 2] = ["0x4bfb41d5b3570defd03c39a9a4d8de6bd8b8982e", "0xc5d563a36ae78145c45a50134d48a1215220f80a"];
pub const USDC_E: &str = "0x2791bca1f2de4661ed88a30c99a7a9449aa84174";
pub const SELL_CAP: &str = "polymarket:sell";
pub const MAX_FEE_BPS: u128 = 200;
pub const MAX_LIFETIME_S: u64 = 30 * 86_400;

#[near(serializers = [json])]
#[derive(Clone, Debug)]
pub struct PmOrder {
    pub exchange: String,
    /// Decimal.
    pub salt: String,
    pub maker: String,
    pub signer: String,
    pub taker: String,
    /// The outcome token (decimal, up to 2^256 - 1).
    pub token_id: String,
    pub maker_amount: U128,
    pub taker_amount: U128,
    /// Unix seconds.
    pub expiration: u64,
    pub nonce: U128,
    pub fee_rate_bps: U128,
    /// 0 buy, 1 sell.
    pub side: u8,
    pub signature_type: u8,
}

fn u_word(v: u128) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[16..].copy_from_slice(&v.to_be_bytes());
    w
}

fn addr_word(a: &[u8; 20]) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(a);
    w
}

pub fn domain_separator(exchange: &[u8; 20]) -> [u8; 32] {
    let mut b = env::keccak256_array(DOMAIN_TYPEHASH).to_vec();
    b.extend(env::keccak256_array(b"Polymarket CTF Exchange"));
    b.extend(env::keccak256_array(b"1"));
    b.extend(u_word(POLYGON as u128));
    b.extend(addr_word(exchange));
    env::keccak256_array(&b)
}

/// The EIP-712 digest the exchange checks the signature against.
pub fn order_hash(o: &PmOrder) -> Result<[u8; 32], &'static str> {
    let mut s = env::keccak256_array(ORDER_TYPEHASH).to_vec();
    s.extend(crate::seaport::dec_word(&o.salt)?);
    s.extend(addr_word(&evm::parse_address(&o.maker)?));
    s.extend(addr_word(&evm::parse_address(&o.signer)?));
    s.extend(addr_word(&evm::parse_address(&o.taker)?));
    s.extend(crate::seaport::dec_word(&o.token_id)?);
    s.extend(u_word(o.maker_amount.0));
    s.extend(u_word(o.taker_amount.0));
    s.extend(u_word(o.expiration as u128));
    s.extend(u_word(o.nonce.0));
    s.extend(u_word(o.fee_rate_bps.0));
    s.extend(u_word(o.side as u128));
    s.extend(u_word(o.signature_type as u128));
    let mut d = vec![0x19, 0x01];
    d.extend(domain_separator(&evm::parse_address(&o.exchange)?));
    d.extend(env::keccak256_array(&s));
    Ok(env::keccak256_array(&d))
}

/// Checks an order against the grant; returns the hash to sign and the spends.
pub fn check(g: &Grant, o: &PmOrder, now_s: u64) -> Result<([u8; 32], Vec<(String, u128)>), &'static str> {
    let ex = evm::fmt_address(&evm::parse_address(&o.exchange)?);
    if !EXCHANGES.contains(&ex.as_str()) {
        return Err("not a Polymarket exchange");
    }
    if !g.evm_venues.iter().any(|v| v.chain_id == POLYGON && v.to.to_lowercase() == ex) {
        return Err("not an allowed venue");
    }
    let me = evm::parse_address(g.evm_self.as_deref().ok_or("this grant doesn't name the account's EVM address")?)?;
    if evm::parse_address(&o.maker)? != me || evm::parse_address(&o.signer)? != me {
        return Err("the account must be maker and signer");
    }
    if o.signature_type != 0 {
        return Err("only the account's own signature (EOA)");
    }
    if evm::parse_address(&o.taker)? != [0; 20] {
        return Err("only public orders");
    }
    if o.fee_rate_bps.0 > MAX_FEE_BPS {
        return Err("fee over 2%");
    }
    if o.expiration <= now_s || o.expiration > now_s + MAX_LIFETIME_S {
        return Err("an order expires within 30 days");
    }
    if o.maker_amount.0 == 0 || o.taker_amount.0 == 0 {
        return Err("amounts must be more than zero");
    }
    let spend = match o.side {
        0 => (format!("evm:{POLYGON}:{USDC_E}"), o.maker_amount.0),
        1 => (SELL_CAP.to_string(), o.maker_amount.0),
        _ => return Err("side is buy (0) or sell (1)"),
    };
    Ok((order_hash(o)?, vec![spend]))
}
