//! The owner sends ADA on Cardano preprod: a Shelley-era transaction body
//! in CBOR (inputs, outputs, fee, TTL), signed by the account's ed25519 key
//! over BLAKE2b-256 of the body, built here, never by a node.

use super::{err, hex32};
use wasm_bindgen::prelude::*;

/// Preprod protocol parameters (Koios `/epoch_params`, checked
/// 9 November 2026): fee = A × size + B, and the minimum ADA an output
/// needs is (160 + its size) × COINS_PER_BYTE.
const MIN_FEE_A: u64 = 44;
const MIN_FEE_B: u64 = 155_381;
const COINS_PER_BYTE: u64 = 4_310;

fn cbor_head(out: &mut Vec<u8>, major: u8, n: u64) {
    let m = major << 5;
    match n {
        0..=23 => out.push(m | n as u8),
        24..=0xff => out.extend([m | 24, n as u8]),
        0x100..=0xffff => {
            out.push(m | 25);
            out.extend((n as u16).to_be_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(m | 26);
            out.extend((n as u32).to_be_bytes());
        }
        _ => {
            out.push(m | 27);
            out.extend(n.to_be_bytes());
        }
    }
}
fn cbor_bytes(out: &mut Vec<u8>, b: &[u8]) {
    cbor_head(out, 2, b.len() as u64);
    out.extend(b);
}

/// A Shelley address on a test network: its raw bytes.
fn address_bytes(a: &str) -> Result<Vec<u8>, String> {
    let bad = || "not a Cardano testnet address (addr_test1…)".to_string();
    let parsed = bech32::primitives::decode::CheckedHrpstring::new::<bech32::Bech32>(a).map_err(|_| bad())?;
    if parsed.hrp().as_str() != "addr_test" {
        return Err(bad());
    }
    let b: Vec<u8> = parsed.byte_iter().collect();
    // Header: type in the high nibble (0–7 are Shelley payment addresses),
    // network ID in the low nibble (0 for every test network).
    if b.is_empty() || b[0] >> 4 > 7 || b[0] & 0x0f != 0 || !(b.len() == 29 || b.len() == 57) {
        return Err(bad());
    }
    Ok(b)
}

/// An addr_test1… address's raw bytes, hex (what the extension's Cardano
/// payment grants list).
pub fn cardano_address_hex(a: &str) -> Result<String, String> {
    Ok(hex::encode(address_bytes(a)?))
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct AdaInput {
    tx_hash: String,
    tx_index: u32,
    lovelace: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct AdaOutput {
    address: String,
    lovelace: u64,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct AdaPlan {
    from: String,
    inputs: Vec<AdaInput>,
    outputs: Vec<AdaOutput>,
    fee: u64,
    ttl: u64,
    /// BLAKE2b-256 of the body: what the key signs, and the txid.
    hash: String,
}

fn body(p: &AdaPlan) -> Result<Vec<u8>, String> {
    let mut b = vec![];
    cbor_head(&mut b, 5, 4);
    cbor_head(&mut b, 0, 0);
    cbor_head(&mut b, 4, p.inputs.len() as u64);
    for i in &p.inputs {
        cbor_head(&mut b, 4, 2);
        cbor_bytes(&mut b, &hex32(&i.tx_hash)?);
        cbor_head(&mut b, 0, i.tx_index as u64);
    }
    cbor_head(&mut b, 0, 1);
    cbor_head(&mut b, 4, p.outputs.len() as u64);
    for o in &p.outputs {
        cbor_head(&mut b, 4, 2);
        cbor_bytes(&mut b, &address_bytes(&o.address)?);
        cbor_head(&mut b, 0, o.lovelace);
    }
    cbor_head(&mut b, 0, 2);
    cbor_head(&mut b, 0, p.fee);
    cbor_head(&mut b, 0, 3);
    cbor_head(&mut b, 0, p.ttl);
    Ok(b)
}

fn blake256(b: &[u8]) -> [u8; 32] {
    blake2b_simd::Params::new().hash_length(32).hash(b).as_bytes().try_into().unwrap()
}

fn min_ada(address: &str) -> Result<u64, String> {
    // An ADA-only output: array(2), address bytes, coin (up to 9 bytes).
    Ok((160 + 2 + address_bytes(address)?.len() as u64 + 2 + 9) * COINS_PER_BYTE)
}

/// The signed transaction around `body`: [body, {0: [[vkey, sig]]}, true, null].
fn signed(body: &[u8], vkey: &[u8; 32], sig: &[u8; 64]) -> Vec<u8> {
    let mut t = vec![];
    cbor_head(&mut t, 4, 4);
    t.extend(body);
    cbor_head(&mut t, 5, 1);
    cbor_head(&mut t, 0, 0);
    cbor_head(&mut t, 4, 1);
    cbor_head(&mut t, 4, 2);
    cbor_bytes(&mut t, vkey);
    cbor_bytes(&mut t, sig);
    t.push(0xf5);
    t.push(0xf6);
    t
}

/// Plans an ADA send: coins largest first (only ADA-only UTxOs; Koios's
/// `[{tx_hash, tx_index, value, asset_list}]`), change back to `from`
/// unless below the minimum ADA, then it goes to the fee. `ttl` is the
/// last slot the transaction is valid in.
#[wasm_bindgen]
pub fn cardano_plan(from: &str, to: &str, lovelace: &str, utxos_json: &str, ttl: u64) -> Result<String, String> {
    let amount: u64 = lovelace.parse().map_err(err)?;
    if amount < min_ada(to)? {
        return Err(format!("Cardano outputs need at least {} ADA", min_ada(to)? as f64 / 1e6));
    }
    address_bytes(from)?;
    let raw: Vec<serde_json::Value> = serde_json::from_str(utxos_json).map_err(err)?;
    let mut utxos: Vec<AdaInput> = raw
        .iter()
        .filter(|u| u["asset_list"].as_array().is_none_or(|a| a.is_empty()))
        .filter_map(|u| {
            Some(AdaInput {
                tx_hash: u["tx_hash"].as_str()?.to_string(),
                tx_index: u["tx_index"].as_u64()? as u32,
                lovelace: u["value"].as_str().and_then(|v| v.parse().ok()).or(u["value"].as_u64())?,
            })
        })
        .collect();
    utxos.sort_by(|a, b| b.lovelace.cmp(&a.lovelace));
    let mut p = AdaPlan { from: from.into(), inputs: vec![], outputs: vec![], fee: 0, ttl, hash: String::new() };
    let fee_of = |p: &AdaPlan| -> Result<u64, String> {
        // A 16-byte margin: encoders that tag sets (CBOR tag 258, 3 bytes
        // each) count a few more bytes than we send.
        let size = signed(&body(p)?, &[0; 32], &[0; 64]).len() as u64 + 16;
        Ok(MIN_FEE_A * size + MIN_FEE_B)
    };
    let change_min = min_ada(from)?;
    let mut total = 0u64;
    for u in utxos {
        if p.inputs.len() == 10 {
            break;
        }
        total += u.lovelace;
        p.inputs.push(u);
        p.outputs = vec![AdaOutput { address: to.into(), lovelace: amount }, AdaOutput { address: from.into(), lovelace: 0 }];
        p.fee = 200_000;
        if total >= amount + fee_of(&p)? + change_min {
            break;
        }
    }
    if p.inputs.is_empty() {
        return Err("no ADA to spend".into());
    }
    // Fees grow with the fee field's own encoding: settle in two passes.
    p.outputs = vec![AdaOutput { address: to.into(), lovelace: amount }, AdaOutput { address: from.into(), lovelace: 1 << 40 }];
    p.fee = 1 << 20;
    let fee = fee_of(&p)?;
    if total >= amount + fee + change_min {
        p.fee = fee;
        p.outputs[1].lovelace = total - amount - fee;
    } else {
        p.outputs.truncate(1);
        p.fee = 1 << 20;
        let fee = fee_of(&p)?;
        if total < amount + fee {
            return Err("not enough ADA for this amount and fee".into());
        }
        p.fee = total - amount; // the small remainder goes to the fee
    }
    p.hash = hex::encode(blake256(&body(&p)?));
    serde_json::to_string(&p).map_err(err)
}

fn checked(plan_json: &str) -> Result<(AdaPlan, Vec<u8>), String> {
    let p: AdaPlan = serde_json::from_str(plan_json).map_err(err)?;
    let b = body(&p)?;
    if hex::encode(blake256(&b)) != p.hash {
        return Err("signing hash doesn't match the transaction".into());
    }
    let total_in: u64 = p.inputs.iter().map(|i| i.lovelace).sum();
    let total_out: u64 = p.outputs.iter().map(|o| o.lovelace).sum();
    if total_in < total_out || total_in - total_out != p.fee {
        return Err("fee doesn't add up".into());
    }
    Ok((p, b))
}

/// For Confirm: what a plan pays, with the body hash recomputed.
#[wasm_bindgen]
pub fn describe_cardano_plan(plan_json: &str) -> Result<String, String> {
    let (p, _) = checked(plan_json)?;
    let pays: Vec<_> = p.outputs.iter().filter(|o| o.address != p.from).map(|o| serde_json::json!({ "to": o.address, "lovelace": o.lovelace.to_string() })).collect();
    serde_json::to_string(&serde_json::json!({ "pays": pays, "fee_lovelace": p.fee.to_string(), "inputs": p.inputs.len(), "ttl": p.ttl, "hash": p.hash })).map_err(err)
}

/// The signed transaction (CBOR hex, for submission) and its txid, after
/// checking the MPC signature (`{ signature: [64] }`) verifies over the body
/// hash for a key whose hash is `from`'s payment key hash.
#[wasm_bindgen]
pub fn cardano_signed_tx(plan_json: &str, mpc_signature_json: &str, derived_public_key: &str) -> Result<String, String> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let (p, b) = checked(plan_json)?;
    let pk: [u8; 32] = bs58::decode(derived_public_key.strip_prefix("ed25519:").ok_or("expected an ed25519 key")?)
        .into_vec()
        .map_err(err)?
        .try_into()
        .map_err(|_| "expected 32 bytes".to_string())?;
    if super::chain_address("cardano", derived_public_key)? != p.from {
        return Err("key isn't this account's Cardano key".into());
    }
    let sig: serde_json::Value = serde_json::from_str(mpc_signature_json).map_err(err)?;
    let bytes: Vec<u8> = sig["signature"].as_array().ok_or("signature")?.iter().map(|v| v.as_u64().unwrap_or(256) as u8).collect();
    let bytes: [u8; 64] = bytes.try_into().map_err(|_| "signature must be 64 bytes".to_string())?;
    let hash = hex32(&p.hash)?;
    VerifyingKey::from_bytes(&pk).map_err(err)?.verify(&hash, &Signature::from_bytes(&bytes)).map_err(|_| "signature is not from this account".to_string())?;
    serde_json::to_string(&serde_json::json!({ "cbor": hex::encode(signed(&b, &pk, &bytes)), "txid": p.hash })).map_err(err)
}
