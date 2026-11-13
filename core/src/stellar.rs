//! Stellar: the owner's XLM sends.
//!
//! One operation per transaction: a native Payment, or CreateAccount when
//! the recipient doesn't exist yet (Stellar refuses a payment to an account
//! that isn't there). The MPC key signs the transaction hash:
//! sha256(sha256(passphrase) || ENVELOPE_TYPE_TX || tx), as the extension
//! does for agents (near/extension/src/stellar.rs). Layout checked against
//! @stellar/stellar-sdk 13 in the tests.

use sha2::{Digest, Sha256};
use wasm_bindgen::prelude::*;

use crate::err;

pub const TESTNET: &str = "Test SDF Network ; September 2015";
/// Highest fee: 0.01 XLM.
const MAX_FEE: u32 = 100_000;

fn base32(s: &str) -> Option<Vec<u8>> {
    let (mut buf, mut bits, mut out) = (0u32, 0u32, vec![]);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'2'..=b'7' => c - b'2' + 26,
            _ => return None,
        } as u32;
        buf = buf << 5 | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn crc16(b: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &x in b {
        crc ^= (x as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { crc << 1 ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

/// A G… account address as its ed25519 key (checksum checked).
pub fn account(a: &str) -> Result<[u8; 32], String> {
    let b = base32(a).ok_or("not a Stellar account")?;
    if b.len() != 35 || b[0] != 6 << 3 || b[33..] != crc16(&b[..33]).to_le_bytes() {
        return Err("not a Stellar account".into());
    }
    Ok(b[1..33].try_into().unwrap())
}

fn key(derived_public_key: &str) -> Result<[u8; 32], String> {
    let b = derived_public_key.strip_prefix("ed25519:").ok_or("expected an ed25519 key")?;
    bs58::decode(b).into_vec().map_err(err)?.try_into().map_err(|_| "expected 32 bytes".to_string())
}

fn u32b(o: &mut Vec<u8>, v: u32) {
    o.extend(v.to_be_bytes())
}

/// The transaction hash the key signs.
fn tx_hash(tx: &[u8]) -> [u8; 32] {
    let mut p = Sha256::digest(TESTNET.as_bytes()).to_vec();
    p.extend(2u32.to_be_bytes()); // ENVELOPE_TYPE_TX
    p.extend(tx);
    Sha256::digest(&p).into()
}

/// The fields a send is built from, and decoded back into for Confirm.
struct Send {
    from: [u8; 32],
    to: [u8; 32],
    stroops: u64,
    create: bool,
    fee: u32,
    seq: u64,
    max_time: u64,
    memo: String,
}

fn encode(s: &Send) -> Vec<u8> {
    let mut o = vec![];
    u32b(&mut o, 0); // source: MuxedAccount KEY_TYPE_ED25519
    o.extend(s.from);
    u32b(&mut o, s.fee);
    o.extend(s.seq.to_be_bytes());
    u32b(&mut o, 1); // PRECOND_TIME: no lower bound, an upper bound
    o.extend(0u64.to_be_bytes());
    o.extend(s.max_time.to_be_bytes());
    if s.memo.is_empty() {
        u32b(&mut o, 0); // MEMO_NONE
    } else {
        u32b(&mut o, 1); // MEMO_TEXT
        u32b(&mut o, s.memo.len() as u32);
        o.extend(s.memo.as_bytes());
        o.extend(vec![0u8; (4 - s.memo.len() % 4) % 4]);
    }
    u32b(&mut o, 1); // one operation
    u32b(&mut o, 0); // no operation source
    if s.create {
        u32b(&mut o, 0); // CREATE_ACCOUNT: AccountID, starting balance
        u32b(&mut o, 0);
        o.extend(s.to);
    } else {
        u32b(&mut o, 1); // PAYMENT: MuxedAccount, native asset, amount
        u32b(&mut o, 0);
        o.extend(s.to);
        u32b(&mut o, 0); // ASSET_TYPE_NATIVE
    }
    o.extend(s.stroops.to_be_bytes());
    u32b(&mut o, 0); // ext v0
    o
}

/// An XLM send from the owner: the transaction XDR (hex) and its hash,
/// which the MPC key signs. `create` when the recipient doesn't exist yet
/// (at least 1 XLM, Stellar's minimum balance).
#[wasm_bindgen]
pub fn stellar_payment(derived_public_key: &str, to: &str, stroops: &str, create: bool, fee: u32, seq_num: &str, max_time: f64, memo: &str) -> Result<String, String> {
    let from = key(derived_public_key)?;
    let to = account(to)?;
    if to == from {
        return Err("sending to yourself".into());
    }
    let stroops: u64 = stroops.parse().map_err(err)?;
    if stroops == 0 || stroops > i64::MAX as u64 {
        return Err("amount out of range".into());
    }
    if create && stroops < 10_000_000 {
        return Err("a new Stellar account needs at least 1 XLM".into());
    }
    if fee == 0 || fee > MAX_FEE {
        return Err("fee outside 1 stroop to 0.01 XLM".into());
    }
    if memo.len() > 28 {
        return Err("memo is at most 28 bytes".into());
    }
    let seq: u64 = seq_num.parse().map_err(err)?;
    let s = Send { from, to, stroops, create, fee, seq, max_time: max_time as u64, memo: memo.into() };
    let tx = encode(&s);
    Ok(serde_json::json!({ "tx_xdr": hex::encode(&tx), "hash": hex::encode(tx_hash(&tx)) }).to_string())
}

/// Decodes a send for Confirm by re-encoding it from its fields: only the
/// exact layout stellar_payment builds is recognised.
#[wasm_bindgen]
pub fn describe_stellar(tx_hex: &str) -> Result<String, String> {
    let t = hex::decode(tx_hex).map_err(err)?;
    let not = || "not a plain XLM send".to_string();
    let u32at = |i: usize| -> Result<u32, String> { Ok(u32::from_be_bytes(t.get(i..i + 4).ok_or_else(not)?.try_into().unwrap())) };
    let u64at = |i: usize| -> Result<u64, String> { Ok(u64::from_be_bytes(t.get(i..i + 8).ok_or_else(not)?.try_into().unwrap())) };
    let k32 = |i: usize| -> Result<[u8; 32], String> { Ok(t.get(i..i + 32).ok_or_else(not)?.try_into().unwrap()) };
    let from = k32(4)?;
    let fee = u32at(36)?;
    let seq = u64at(40)?;
    let max_time = u64at(48 + 4 + 8)?;
    let mut i = 68;
    let memo = match u32at(i)? {
        0 => {
            i += 4;
            String::new()
        }
        1 => {
            let n = u32at(i + 4)? as usize;
            if n > 28 {
                return Err(not());
            }
            let m = String::from_utf8(t.get(i + 8..i + 8 + n).ok_or_else(not)?.to_vec()).map_err(|_| not())?;
            i += 8 + n + (4 - n % 4) % 4;
            m
        }
        _ => return Err(not()),
    };
    let create = match u32at(i + 8)? {
        0 => true,
        1 => false,
        _ => return Err(not()),
    };
    let to = k32(i + 16)?;
    let stroops = u64at(i + 48 + if create { 0 } else { 4 })?;
    let s = Send { from, to, stroops, create, fee, seq, max_time, memo };
    if encode(&s) != t {
        return Err(not());
    }
    Ok(serde_json::json!({
        "kind": if create { "stellar_create_account" } else { "stellar_payment" },
        "to": strkey(&to), "stroops": stroops.to_string(), "fee_stroops": fee, "memo": s.memo,
        "hash": hex::encode(tx_hash(&t)),
    })
    .to_string())
}

fn strkey(pk: &[u8; 32]) -> String {
    crate::chain_address("stellar", &format!("ed25519:{}", bs58::encode(pk).into_string())).unwrap_or_default()
}

/// The signed envelope (base64, for Horizon's /transactions), after
/// checking the MPC signature verifies for the account over the hash.
#[wasm_bindgen]
pub fn stellar_signed_tx(tx_hex: &str, mpc_signature_json: &str, derived_public_key: &str) -> Result<String, String> {
    use base64::Engine;
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let pk = key(derived_public_key)?;
    let tx = hex::decode(tx_hex).map_err(err)?;
    let sig: serde_json::Value = serde_json::from_str(mpc_signature_json).map_err(err)?;
    let bytes: Vec<u8> = sig["signature"].as_array().ok_or("signature")?.iter().map(|v| v.as_u64().unwrap_or(256) as u8).collect();
    let bytes: [u8; 64] = bytes.try_into().map_err(|_| "signature must be 64 bytes".to_string())?;
    VerifyingKey::from_bytes(&pk).map_err(err)?.verify(&tx_hash(&tx), &Signature::from_bytes(&bytes)).map_err(|_| "signature is not from this account".to_string())?;
    if tx.get(4..36) != Some(&pk[..]) {
        return Err("the transaction is not from this account".into());
    }
    let mut e = vec![];
    u32b(&mut e, 2); // ENVELOPE_TYPE_TX
    e.extend(&tx);
    u32b(&mut e, 1); // one signature: hint (last 4 key bytes), then 64 bytes
    e.extend(&pk[28..]);
    u32b(&mut e, 64);
    e.extend(bytes);
    Ok(base64::engine::general_purpose::STANDARD.encode(e))
}
