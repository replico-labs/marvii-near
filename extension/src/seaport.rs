//! Seaport 1.6 listings (OpenSea's marketplace contract): the agent asks
//! for an order to be signed; the extension checks it and builds the
//! EIP-712 hash itself, which the account's EVM key signs off-chain.
//!
//! A listing the extension signs:
//! - the offerer is the account (`evm_self`), with no zone, zone hash or
//!   conduit (the account approves Seaport itself), FULL_OPEN;
//! - offers exactly one ERC-721 from a contract the grant lists as a venue;
//! - asks only for the chain's coin, at fixed prices (no Dutch auctions),
//!   with at least the grant's `evm_nft_floor` to the account and at most a
//!   tenth of that to anyone else (marketplace fees and royalties);
//! - lasts at most 180 days.
//! Each one counts 1 against `nft:evm:<chain id>`. Buying and cancelling are
//! ordinary transactions to Seaport (evm.rs, rules.rs).
//!
//! Typehashes and the domain from Seaport's own type strings (`cast keccak`);
//! the domain separator matches what `information()` returns on Base
//! Sepolia (tests).

use near_sdk::{env, json_types::U128, near};

use crate::evm;
use crate::rules::Grant;

/// keccak256 of the type strings, as Seaport 1.6 defines them.
const OFFER_ITEM_TYPEHASH: &str = "a66999307ad1bb4fde44d13a5d710bd7718e0c87c1eef68a571629fbf5b93d02";
const CONSIDERATION_ITEM_TYPEHASH: &str = "42d81c6929ffdc4eb27a0808e40e82516ad42296c166065de7f812492304ff6e";
const ORDER_COMPONENTS_TYPEHASH: &str = "fa445660b7e21515a59617fcd68910b487aa5808b8abda3d78bc85df364b2c2f";
const EIP712_DOMAIN_TYPEHASH: &str = "8b73c3c69bb8fe3d512ecc4cf759cc79239f7b179b0ffacaa9a75d522b39400f";
/// The longest a listing may stay open.
pub const MAX_DURATION_S: u64 = 180 * 86_400;
const ITEM_NATIVE: u8 = 0;
const ITEM_ERC721: u8 = 2;

#[near(serializers = [json])]
#[derive(Clone, Debug)]
pub struct SeaportItem {
    pub item_type: u8,
    pub token: String,
    /// Token ID (decimal, up to 2^256 - 1).
    pub identifier: String,
    pub start_amount: U128,
    pub end_amount: U128,
    /// Consideration items only: who is paid.
    #[serde(default)]
    pub recipient: Option<String>,
}

#[near(serializers = [json])]
#[derive(Clone, Debug)]
pub struct SeaportOrder {
    pub chain_id: u64,
    /// The Seaport contract (the EIP-712 verifying contract).
    pub seaport: String,
    pub offerer: String,
    pub zone: String,
    pub offer: Vec<SeaportItem>,
    pub consideration: Vec<SeaportItem>,
    pub order_type: u8,
    pub start_time: u64,
    pub end_time: u64,
    /// 32 bytes, hex.
    pub zone_hash: String,
    /// Decimal.
    pub salt: String,
    /// 32 bytes, hex.
    pub conduit_key: String,
    /// The offerer's Seaport counter (getCounter), decimal.
    pub counter: String,
}

/// A decimal string as a 256-bit big-endian word.
pub fn dec_word(s: &str) -> Result<[u8; 32], &'static str> {
    if s.is_empty() || s.len() > 78 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err("not a decimal number");
    }
    let mut w = [0u8; 32];
    for b in s.bytes() {
        // w = w * 10 + digit
        let mut carry = (b - b'0') as u32;
        for x in w.iter_mut().rev() {
            let v = *x as u32 * 10 + carry;
            *x = v as u8;
            carry = v >> 8;
        }
        if carry != 0 {
            return Err("number too large");
        }
    }
    Ok(w)
}

fn hex32(s: &str) -> Result<[u8; 32], &'static str> {
    evm::parse_hex(s)?.try_into().map_err(|_| "expected 32 bytes")
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

fn item_hash(i: &SeaportItem, consideration: bool) -> Result<[u8; 32], &'static str> {
    let mut b = Vec::with_capacity(32 * 7);
    b.extend(hex::decode(if consideration { CONSIDERATION_ITEM_TYPEHASH } else { OFFER_ITEM_TYPEHASH }).unwrap());
    b.extend(u_word(i.item_type as u128));
    b.extend(addr_word(&evm::parse_address(&i.token)?));
    b.extend(dec_word(&i.identifier)?);
    b.extend(u_word(i.start_amount.0));
    b.extend(u_word(i.end_amount.0));
    if consideration {
        b.extend(addr_word(&evm::parse_address(i.recipient.as_deref().ok_or("a consideration item needs a recipient")?)?));
    }
    Ok(env::keccak256_array(&b))
}

/// The EIP-712 digest Seaport checks the offerer's signature against.
pub fn order_hash(o: &SeaportOrder) -> Result<[u8; 32], &'static str> {
    let list = |items: &[SeaportItem], c: bool| -> Result<[u8; 32], &'static str> {
        let mut b = vec![];
        for i in items {
            b.extend(item_hash(i, c)?);
        }
        Ok(env::keccak256_array(&b))
    };
    let mut s = hex::decode(ORDER_COMPONENTS_TYPEHASH).unwrap();
    s.extend(addr_word(&evm::parse_address(&o.offerer)?));
    s.extend(addr_word(&evm::parse_address(&o.zone)?));
    s.extend(list(&o.offer, false)?);
    s.extend(list(&o.consideration, true)?);
    s.extend(u_word(o.order_type as u128));
    s.extend(u_word(o.start_time as u128));
    s.extend(u_word(o.end_time as u128));
    s.extend(hex32(&o.zone_hash)?);
    s.extend(dec_word(&o.salt)?);
    s.extend(hex32(&o.conduit_key)?);
    s.extend(dec_word(&o.counter)?);
    let struct_hash = env::keccak256_array(&s);
    let mut d = vec![0x19, 0x01];
    d.extend(domain_separator(o.chain_id, &evm::parse_address(&o.seaport)?));
    d.extend(struct_hash);
    Ok(env::keccak256_array(&d))
}

pub fn domain_separator(chain_id: u64, seaport: &[u8; 20]) -> [u8; 32] {
    let mut b = hex::decode(EIP712_DOMAIN_TYPEHASH).unwrap();
    b.extend(env::keccak256_array(b"Seaport"));
    b.extend(env::keccak256_array(b"1.6"));
    b.extend(u_word(chain_id as u128));
    b.extend(addr_word(seaport));
    env::keccak256_array(&b)
}

/// Checks a listing against the grant; returns the hash to sign and the spends.
pub fn check(g: &Grant, o: &SeaportOrder, queued: bool) -> Result<([u8; 32], Vec<(String, u128)>), &'static str> {
    let chain = o.chain_id;
    let venue = |a: &str| -> Result<bool, &'static str> {
        let a = evm::fmt_address(&evm::parse_address(a)?);
        Ok(g.evm_venues.iter().any(|v| v.chain_id == chain && v.to.to_lowercase() == a))
    };
    if !queued && !venue(&o.seaport)? {
        return Err("not an allowed venue");
    }
    let me = evm::parse_address(g.evm_self.as_deref().ok_or("this grant doesn't name the account's EVM address")?)?;
    if evm::parse_address(&o.offerer)? != me {
        return Err("the account must be the offerer");
    }
    if evm::parse_address(&o.zone)? != [0; 20] || hex32(&o.zone_hash)? != [0; 32] || o.order_type != 0 {
        return Err("only plain full-open orders, no zone");
    }
    if hex32(&o.conduit_key)? != [0; 32] {
        return Err("no conduit: approve Seaport itself");
    }
    if o.end_time <= o.start_time || o.end_time - o.start_time > MAX_DURATION_S {
        return Err("a listing lasts at most 180 days");
    }
    let [nft] = &o.offer[..] else { return Err("offer exactly one NFT") };
    if nft.item_type != ITEM_ERC721 || nft.start_amount.0 != 1 || nft.end_amount.0 != 1 {
        return Err("offer exactly one NFT");
    }
    if !queued && !venue(&nft.token)? {
        return Err("that NFT contract isn't allowed");
    }
    if o.consideration.is_empty() || o.consideration.len() > 4 {
        return Err("1 to 4 payments");
    }
    let (mut mine, mut others) = (0u128, 0u128);
    for c in &o.consideration {
        if c.item_type != ITEM_NATIVE || evm::parse_address(&c.token)? != [0; 20] || dec_word(&c.identifier)? != [0; 32] {
            return Err("only payment in the chain's coin");
        }
        if c.start_amount.0 != c.end_amount.0 {
            return Err("fixed prices only");
        }
        let to = evm::parse_address(c.recipient.as_deref().ok_or("a consideration item needs a recipient")?)?;
        let t = if to == me { &mut mine } else { &mut others };
        *t = t.checked_add(c.start_amount.0).ok_or("overflow")?;
    }
    if g.evm_nft_floor.0 == 0 || mine < g.evm_nft_floor.0 {
        return Err("listing price below the floor");
    }
    if others > mine / 10 {
        return Err("fees over a tenth of the price");
    }
    Ok((order_hash(o)?, vec![(format!("nft:evm:{chain}"), 1)]))
}
