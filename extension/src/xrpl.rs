//! XRP Ledger transactions for Chain Signatures (secp256k1, path `xrp`).
//!
//! The agent sends typed fields; the extension builds the canonical binary
//! itself and signs `sha512half("STX\0" || fields)`. The ledger's own order
//! book, AMMs and NFTs are all native transaction types, so every rule is
//! checked on fields the extension serialises, never on bytes it was handed.
//!
//! Field and type codes from ripple-binary-codec's definitions.json (v2).

use near_sdk::{env, json_types::U64, near};
use sha2::{Digest, Sha512};

use crate::rules::XrplGrant;

/// Highest fee: 0.1 XRP (the network's normal fee is 12 drops).
pub const MAX_FEE: u64 = 100_000;
/// Offer flags the agent may set: passive, immediate-or-cancel, fill-or-kill, sell.
const OFFER_FLAGS: u32 = 0x0001_0000 | 0x0002_0000 | 0x0004_0000 | 0x0008_0000;
const TF_SELL_NFT: u32 = 0x0000_0001;
const TF_TRANSFERABLE: u32 = 0x0000_0008;
const TF_SET_NO_RIPPLE: u32 = 0x0002_0000;
const TF_SINGLE_ASSET: u32 = 0x0008_0000;
const TF_TWO_ASSET: u32 = 0x0010_0000;
const TF_WITHDRAW_ALL: u32 = 0x0002_0000;

/// An amount: `currency` "XRP" with `value` in drops, or an issued
/// currency (3 letters or 40 hex) with its issuer and a decimal value.
#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XAmount {
    pub currency: String,
    #[serde(default)]
    pub issuer: Option<String>,
    pub value: String,
}

/// An asset without an amount (AMM pools): XRP, or a currency and issuer.
#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XIssue {
    pub currency: String,
    #[serde(default)]
    pub issuer: Option<String>,
}

#[near(serializers = [json])]
#[derive(Clone, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum XrplOp {
    Payment { destination: String, amount: XAmount, #[serde(default)] destination_tag: Option<u32> },
    /// Order book: the account gives `taker_gets` for `taker_pays`.
    OfferCreate { taker_pays: XAmount, taker_gets: XAmount, #[serde(default)] flags: u32 },
    OfferCancel { offer_sequence: u32 },
    /// Trust line to a listed issued currency (with no-ripple set).
    TrustSet { limit: XAmount },
    /// AMM deposit: one asset, or both.
    AmmDeposit { asset: XIssue, asset2: XIssue, amount: XAmount, #[serde(default)] amount2: Option<XAmount> },
    /// AMM withdraw everything the account holds in the pool.
    AmmWithdrawAll { asset: XIssue, asset2: XIssue },
    /// Mint a transferable NFT with no transfer fee.
    NftMint { taxon: u32, #[serde(default)] uri: Option<String> },
    /// Offer one of the account's NFTs for sale.
    NftSellOffer { nftoken_id: String, amount: XAmount },
    /// Offer to buy an NFT from its owner.
    NftBuyOffer { nftoken_id: String, owner: String, amount: XAmount },
    /// Accept a buy offer for one of the account's NFTs.
    NftAcceptBuyOffer { buy_offer: String },
    /// Cancel the account's own NFT offers (listings or bids).
    NftCancelOffers { offers: Vec<String> },
}

#[near(serializers = [json])]
#[derive(Clone, Debug)]
pub struct XrplTx {
    pub sequence: u32,
    pub fee: U64,
    pub last_ledger_sequence: u32,
    pub op: XrplOp,
}

pub fn account_id(a: &str) -> Result<[u8; 20], &'static str> {
    let b = bs58::decode(a).with_alphabet(bs58::Alphabet::RIPPLE).into_vec().map_err(|_| "not an XRP address")?;
    if b.len() != 25 || b[0] != 0 {
        return Err("not an XRP address");
    }
    let check = env::sha256_array(&env::sha256_array(&b[..21]));
    if check[..4] != b[21..] {
        return Err("not an XRP address");
    }
    Ok(b[1..21].try_into().unwrap())
}

/// The account ID of a compressed secp256k1 key: ripemd160(sha256(key)).
pub fn key_account(pk: &[u8]) -> [u8; 20] {
    env::ripemd160_array(&env::sha256_array(pk))
}

fn currency(c: &str) -> Result<[u8; 20], &'static str> {
    let mut out = [0u8; 20];
    if c.len() == 3 && c.bytes().all(|b| b.is_ascii_alphanumeric()) && c != "XRP" {
        out[12..15].copy_from_slice(c.as_bytes());
        return Ok(out);
    }
    if c.len() == 40 {
        let v = hex::decode(c).map_err(|_| "bad currency")?;
        if v[0] == 0 {
            return Err("bad currency");
        }
        out.copy_from_slice(&v);
        return Ok(out);
    }
    Err("bad currency")
}

/// The cap asset name: `xrp`, or `xrpl:<currency>.<issuer>`.
pub fn asset_name(currency: &str, issuer: &Option<String>) -> String {
    match issuer {
        None => "xrp".into(),
        Some(i) => format!("xrpl:{currency}.{i}"),
    }
}

/// A decimal string as (mantissa, exponent), normalised as the ledger does.
fn decimal(v: &str) -> Result<(u64, i32), &'static str> {
    let (int, frac) = v.split_once('.').unwrap_or((v, ""));
    if int.is_empty() && frac.is_empty() || !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit()) {
        return Err("bad amount");
    }
    let digits = format!("{int}{frac}");
    let digits = digits.trim_start_matches('0');
    let mut exp = -(frac.len() as i32);
    let digits = {
        let t = digits.trim_end_matches('0');
        exp += (digits.len() - t.len()) as i32;
        t
    };
    if digits.is_empty() {
        return Err("amount must be more than zero");
    }
    if digits.len() > 16 {
        return Err("too many digits");
    }
    let mut m: u64 = digits.parse().map_err(|_| "bad amount")?;
    while m < 1_000_000_000_000_000 {
        m *= 10;
        exp -= 1;
    }
    if !(-96..=80).contains(&exp) {
        return Err("amount out of range");
    }
    Ok((m, exp))
}

/// The amount in millionths of a unit, for caps (issued currencies only).
pub fn micro(v: &str) -> Result<u128, &'static str> {
    let (m, e) = decimal(v)?;
    let e = e + 6;
    if e >= 0 {
        (m as u128).checked_mul(10u128.checked_pow(e as u32).ok_or("amount out of range")?).ok_or("amount out of range")
    } else {
        let d = 10u128.checked_pow((-e) as u32).ok_or("more than 6 decimals")?;
        if (m as u128) % d != 0 {
            return Err("more than 6 decimals");
        }
        Ok(m as u128 / d)
    }
}

/// The amount's encoding and its spend (asset, units).
fn amount(a: &XAmount) -> Result<(Vec<u8>, String, u128), &'static str> {
    if a.currency == "XRP" {
        if a.issuer.is_some() {
            return Err("XRP has no issuer");
        }
        let drops: u64 = a.value.parse().map_err(|_| "XRP amounts are in drops")?;
        if drops == 0 || drops > 100_000_000_000_000_000 {
            return Err("bad XRP amount");
        }
        return Ok(((0x4000_0000_0000_0000u64 | drops).to_be_bytes().to_vec(), "xrp".into(), drops as u128));
    }
    let issuer = a.issuer.as_deref().ok_or("issued currencies need an issuer")?;
    let (m, e) = decimal(&a.value)?;
    let head: u64 = (1 << 63) | (1 << 62) | (((e + 97) as u64) << 54) | m;
    let mut out = head.to_be_bytes().to_vec();
    out.extend(currency(&a.currency)?);
    out.extend(account_id(issuer)?);
    Ok((out, asset_name(&a.currency, &a.issuer), micro(&a.value)?))
}

fn issue(i: &XIssue) -> Result<Vec<u8>, &'static str> {
    if i.currency == "XRP" {
        if i.issuer.is_some() {
            return Err("XRP has no issuer");
        }
        return Ok(vec![0u8; 20]);
    }
    let mut out = currency(&i.currency)?.to_vec();
    out.extend(account_id(i.issuer.as_deref().ok_or("issued currencies need an issuer")?)?);
    Ok(out)
}

fn hash256(h: &str) -> Result<Vec<u8>, &'static str> {
    hex::decode(h).ok().filter(|v| v.len() == 32).ok_or("expected 32 bytes hex")
}

/// Fields as (type code, field code, bytes), emitted sorted.
struct Fields(Vec<(u8, u8, Vec<u8>)>);

impl Fields {
    fn u16(&mut self, f: u8, v: u16) { self.0.push((1, f, v.to_be_bytes().to_vec())) }
    fn u32(&mut self, f: u8, v: u32) { self.0.push((2, f, v.to_be_bytes().to_vec())) }
    fn h256(&mut self, f: u8, v: Vec<u8>) { self.0.push((5, f, v)) }
    fn amt(&mut self, f: u8, v: Vec<u8>) { self.0.push((6, f, v)) }
    fn blob(&mut self, f: u8, v: &[u8]) {
        let mut b = vec![v.len() as u8];
        b.extend_from_slice(v);
        self.0.push((7, f, b))
    }
    fn acct(&mut self, f: u8, v: &[u8; 20]) {
        let mut b = vec![20u8];
        b.extend_from_slice(v);
        self.0.push((8, f, b))
    }
    fn issue(&mut self, f: u8, v: Vec<u8>) { self.0.push((24, f, v)) }
    fn vec256(&mut self, f: u8, items: &[Vec<u8>]) {
        let mut b = vec![(items.len() * 32) as u8];
        for i in items {
            b.extend(i);
        }
        self.0.push((19, f, b))
    }

    fn bytes(mut self) -> Vec<u8> {
        self.0.sort_by_key(|(t, f, _)| (*t, *f));
        let mut out = vec![];
        for (t, f, v) in self.0 {
            match (t < 16, f < 16) {
                (true, true) => out.push(t << 4 | f),
                (true, false) => out.extend([t << 4, f]),
                (false, true) => out.extend([f, t]),
                (false, false) => out.extend([0, t, f]),
            }
            out.extend(v);
        }
        out
    }
}

/// Checks the transaction against the grant. Returns the unsigned binary
/// (with the signing key), its signing hash, and its spends.
pub fn check(g: &XrplGrant, tx: &XrplTx, queued: bool) -> Result<(Vec<u8>, [u8; 32], Vec<(String, u128)>), &'static str> {
    let me = account_id(&g.self_address)?;
    let pk = hex::decode(&g.public_key).map_err(|_| "bad key")?;
    let fee = tx.fee.0;
    if fee == 0 || fee > MAX_FEE {
        return Err("fee outside 1 drop to 0.1 XRP");
    }
    let allowed = |cur: &str, issuer: &Option<String>| match issuer {
        None => cur == "XRP",
        Some(i) => g.assets.iter().any(|a| *a == format!("{cur}.{i}")),
    };
    let check_amount = |a: &XAmount| if allowed(&a.currency, &a.issuer) { Ok(()) } else { Err("asset not allowed") };
    let check_issue = |i: &XIssue| if allowed(&i.currency, &i.issuer) { Ok(()) } else { Err("asset not allowed") };
    let mut f = Fields(vec![]);
    let mut spends: Vec<(String, u128)> = vec![("xrp".into(), fee as u128)];
    let spend = |s: &mut Vec<(String, u128)>, a: String, v: u128| match s.iter_mut().find(|(x, _)| *x == a) {
        Some((_, t)) => *t += v,
        None => s.push((a, v)),
    };
    let (ty, flags): (u16, u32) = match &tx.op {
        XrplOp::Payment { destination, amount: a, destination_tag } => {
            check_amount(a)?;
            let to = account_id(destination)?;
            if to == me {
                return Err("payment to itself");
            }
            if !queued && !g.recipients.contains(destination) {
                return Err("recipient not allowed");
            }
            let (b, asset, v) = amount(a)?;
            f.amt(1, b);
            f.acct(3, &to);
            if let Some(t) = destination_tag {
                f.u32(14, *t);
            }
            spend(&mut spends, asset, v);
            (0, 0)
        }
        XrplOp::OfferCreate { taker_pays, taker_gets, flags } => {
            if !g.dex {
                return Err("order book not allowed");
            }
            check_amount(taker_pays)?;
            check_amount(taker_gets)?;
            if flags & !OFFER_FLAGS != 0 {
                return Err("offer flag not allowed");
            }
            let (pays, _, _) = amount(taker_pays)?;
            let (gets, asset, v) = amount(taker_gets)?;
            f.amt(4, pays);
            f.amt(5, gets);
            spend(&mut spends, asset, v);
            (7, *flags)
        }
        XrplOp::OfferCancel { offer_sequence } => {
            f.u32(25, *offer_sequence);
            (8, 0)
        }
        XrplOp::TrustSet { limit } => {
            if limit.currency == "XRP" {
                return Err("no trust line for XRP");
            }
            check_amount(limit)?;
            let (b, _, _) = amount(limit)?;
            f.amt(3, b);
            (20, TF_SET_NO_RIPPLE)
        }
        XrplOp::AmmDeposit { asset, asset2, amount: a, amount2 } => {
            if !g.amm {
                return Err("AMMs not allowed");
            }
            check_issue(asset)?;
            check_issue(asset2)?;
            check_amount(a)?;
            f.issue(3, issue(asset)?);
            f.issue(4, issue(asset2)?);
            let (b, n, v) = amount(a)?;
            f.amt(1, b);
            spend(&mut spends, n, v);
            let flags = match amount2 {
                Some(a2) => {
                    check_amount(a2)?;
                    let (b, n, v) = amount(a2)?;
                    f.amt(11, b);
                    spend(&mut spends, n, v);
                    TF_TWO_ASSET
                }
                None => TF_SINGLE_ASSET,
            };
            (36, flags)
        }
        XrplOp::AmmWithdrawAll { asset, asset2 } => {
            if !g.amm {
                return Err("AMMs not allowed");
            }
            check_issue(asset)?;
            check_issue(asset2)?;
            f.issue(3, issue(asset)?);
            f.issue(4, issue(asset2)?);
            (37, TF_WITHDRAW_ALL)
        }
        XrplOp::NftMint { taxon, uri } => {
            if !g.nft {
                return Err("NFTs not allowed");
            }
            f.u32(42, *taxon);
            if let Some(u) = uri {
                let u = hex::decode(u).map_err(|_| "URI must be hex")?;
                if u.is_empty() || u.len() > 256 {
                    return Err("URI must be 1 to 256 bytes");
                }
                // Longer than 192 bytes would need a 2-byte length.
                if u.len() > 192 {
                    return Err("URI longer than 192 bytes");
                }
                f.blob(5, &u);
            }
            (25, TF_TRANSFERABLE)
        }
        XrplOp::NftSellOffer { nftoken_id, amount: a } => {
            if !g.nft {
                return Err("NFTs not allowed");
            }
            check_amount(a)?;
            f.h256(10, hash256(nftoken_id)?);
            f.amt(1, amount(a)?.0);
            (27, TF_SELL_NFT)
        }
        XrplOp::NftBuyOffer { nftoken_id, owner, amount: a } => {
            if !g.nft {
                return Err("NFTs not allowed");
            }
            check_amount(a)?;
            let o = account_id(owner)?;
            if o == me {
                return Err("buying from itself");
            }
            f.h256(10, hash256(nftoken_id)?);
            f.acct(2, &o);
            let (b, n, v) = amount(a)?;
            f.amt(1, b);
            spend(&mut spends, n, v);
            (27, 0)
        }
        XrplOp::NftAcceptBuyOffer { buy_offer } => {
            if !g.nft {
                return Err("NFTs not allowed");
            }
            f.h256(28, hash256(buy_offer)?);
            (29, 0)
        }
        XrplOp::NftCancelOffers { offers } => {
            if !g.nft {
                return Err("NFTs not allowed");
            }
            // The ledger only lets an offer's owner cancel it (or anyone, once expired).
            if offers.is_empty() || offers.len() > 4 {
                return Err("1 to 4 offers");
            }
            let o = offers.iter().map(|h| hash256(h)).collect::<Result<Vec<_>, _>>()?;
            f.vec256(4, &o);
            (28, 0)
        }
    };
    f.u16(2, ty);
    f.u32(2, flags);
    f.u32(4, tx.sequence);
    f.u32(27, tx.last_ledger_sequence);
    f.amt(8, (0x4000_0000_0000_0000u64 | fee).to_be_bytes().to_vec());
    f.blob(3, &pk);
    f.acct(1, &me);
    let bytes = f.bytes();
    let mut h = Sha512::new();
    h.update(b"STX\0");
    h.update(&bytes);
    let hash: [u8; 32] = h.finalize()[..32].try_into().unwrap();
    Ok((bytes, hash, spends))
}
