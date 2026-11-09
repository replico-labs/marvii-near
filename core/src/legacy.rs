//! The owner sends on the Bitcoin-family chains that use P2PKH addresses:
//! Dogecoin testnet, Bitcoin Cash chipnet and Zcash testnet (transparent).
//!
//! Each chain signs a different digest:
//! - Dogecoin: the original Bitcoin digest (the input's scriptSig replaced
//!   by the spent script, SIGHASH_ALL), version 1 transactions.
//! - Bitcoin Cash: BIP-143 with SIGHASH_ALL | SIGHASH_FORKID (0x41).
//! - Zcash: ZIP-244 (v5 transactions, BLAKE2b-256), committing to the
//!   chain's consensus branch ID and an expiry height.
//!
//! As for Bitcoin, the plan carries its own signing hashes, and Confirm and
//! the final assembly both recompute them from the parts.

use super::{BtcInput, BtcOutput, err, hash160, hex32, sha256d, txid_bytes, varint};
use wasm_bindgen::prelude::*;

#[derive(Clone, Copy, PartialEq)]
enum Chain {
    Doge,
    Bch,
    Zec,
}

impl Chain {
    fn parse(s: &str) -> Result<Self, String> {
        Ok(match s {
            "dogecoin" => Chain::Doge,
            "bitcoin-cash" => Chain::Bch,
            "zcash" => Chain::Zec,
            _ => return Err(format!("{s} isn't a P2PKH chain")),
        })
    }
    fn id(self) -> &'static str {
        match self {
            Chain::Doge => "dogecoin",
            Chain::Bch => "bitcoin-cash",
            Chain::Zec => "zcash",
        }
    }
    fn sym(self) -> &'static str {
        match self {
            Chain::Doge => "DOGE",
            Chain::Bch => "BCH",
            Chain::Zec => "ZEC",
        }
    }
    /// The byte appended to each DER signature.
    fn hash_type(self) -> u8 {
        match self {
            Chain::Bch => 0x41,
            _ => 0x01,
        }
    }
    /// Outputs below this go to the fee instead. Dogecoin Core's default
    /// dust limit is 0.01 DOGE; Bitcoin Cash Node's 546 satoshis; Zcash's
    /// is far lower, 10,000 zatoshis is deliberately high.
    fn dust(self) -> u64 {
        match self {
            Chain::Doge => 1_000_000,
            Chain::Bch => 546,
            Chain::Zec => 10_000,
        }
    }
}

const SEQUENCE: u32 = 0xffff_ffff;

/// Zcash v5: header (version 5 with fOverwintered) and version group ID.
const ZEC_V5_HEADER: u32 = 0x8000_0005;
const ZEC_V5_GROUP: u32 = 0x26A7_270A;

fn p2pkh(h: &[u8]) -> Vec<u8> {
    [&[0x76, 0xa9, 0x14][..], h, &[0x88, 0xac]].concat()
}
fn p2sh(h: &[u8]) -> Vec<u8> {
    [&[0xa9, 0x14][..], h, &[0x87]].concat()
}

/// CashAddr decode (Bitcoin Cash): prefix `bchtest`, checksum checked.
fn cashaddr_decode(a: &str) -> Option<(u8, Vec<u8>)> {
    const CHARSET: &[u8] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";
    let a = a.to_lowercase();
    let body = a.strip_prefix("bchtest:").unwrap_or(&a);
    let mut five = vec![];
    for c in body.bytes() {
        five.push(CHARSET.iter().position(|x| *x == c)? as u8);
    }
    if five.len() < 9 {
        return None;
    }
    let mut data: Vec<u8> = b"bchtest".iter().map(|b| b & 31).collect();
    data.push(0);
    data.extend(&five);
    if super::cashaddr_polymod(&data) != 0 {
        return None;
    }
    let payload5 = &five[..five.len() - 8];
    let (mut acc, mut bits, mut out) = (0u32, 0u32, vec![]);
    for d in payload5 {
        acc = (acc << 5) | *d as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    if out.len() != 21 {
        return None;
    }
    Some((out[0], out[1..].to_vec()))
}

/// The output script for a testnet address on `chain`.
fn script(chain: Chain, address: &str) -> Result<Vec<u8>, String> {
    let bad = || format!("not a {} testnet address", match chain { Chain::Doge => "Dogecoin", Chain::Bch => "Bitcoin Cash", Chain::Zec => "Zcash" });
    match chain {
        Chain::Bch => match cashaddr_decode(address).ok_or_else(bad)? {
            (0, h) => Ok(p2pkh(&h)),
            (8, h) => Ok(p2sh(&h)),
            _ => Err(bad()),
        },
        Chain::Doge => {
            let raw = bs58::decode(address).with_check(None).into_vec().map_err(|_| bad())?;
            match (raw.len(), raw.first()) {
                (21, Some(0x71)) => Ok(p2pkh(&raw[1..])),
                (21, Some(0xc4)) => Ok(p2sh(&raw[1..])),
                _ => Err(bad()),
            }
        }
        Chain::Zec => {
            // ZIP-320 TEX address: a P2PKH that only takes transparent funds.
            if address.to_lowercase().starts_with("textest1") {
                let (hrp, data) = bech32::decode(address).map_err(|_| bad())?;
                if hrp.as_str() != "textest" || data.len() != 20 {
                    return Err(bad());
                }
                return Ok(p2pkh(&data));
            }
            let raw = bs58::decode(address).with_check(None).into_vec().map_err(|_| bad())?;
            match (raw.len(), raw.get(..2)) {
                (22, Some([0x1d, 0x25])) => Ok(p2pkh(&raw[2..])),
                (22, Some([0x1c, 0xba])) => Ok(p2sh(&raw[2..])),
                _ => Err(bad()),
            }
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Plan {
    chain: String,
    from: String,
    inputs: Vec<BtcInput>,
    outputs: Vec<BtcOutput>,
    fee: u64,
    /// Zcash only: the consensus branch ID and expiry height the
    /// signatures commit to.
    #[serde(default)]
    branch_id: u32,
    #[serde(default)]
    expiry_height: u32,
    sighashes: Vec<String>,
}

fn outputs_bytes(chain: Chain, outputs: &[BtcOutput]) -> Result<Vec<u8>, String> {
    let mut out = vec![];
    for o in outputs {
        out.extend(o.value.to_le_bytes());
        let s = script(chain, &o.address)?;
        varint(&mut out, s.len());
        out.extend(s);
    }
    Ok(out)
}

fn blake(personal: &[u8; 16], data: &[u8]) -> [u8; 32] {
    let h = blake2b_simd::Params::new().hash_length(32).personal(personal).hash(data);
    h.as_bytes().try_into().unwrap()
}

/// ZIP-244 parts shared by the txid and every signature digest.
struct ZecDigests {
    header: [u8; 32],
    prevouts: [u8; 32],
    sequence: [u8; 32],
    outputs: [u8; 32],
    sapling: [u8; 32],
    orchard: [u8; 32],
}

fn zec_digests(p: &Plan) -> Result<ZecDigests, String> {
    let mut header = vec![];
    for v in [ZEC_V5_HEADER, ZEC_V5_GROUP, p.branch_id, 0, p.expiry_height] {
        header.extend(v.to_le_bytes());
    }
    let mut prevouts = vec![];
    let mut seqs = vec![];
    for i in &p.inputs {
        prevouts.extend(txid_bytes(&i.txid)?);
        prevouts.extend(i.vout.to_le_bytes());
        seqs.extend(SEQUENCE.to_le_bytes());
    }
    Ok(ZecDigests {
        header: blake(b"ZTxIdHeadersHash", &header),
        prevouts: blake(b"ZTxIdPrevoutHash", &prevouts),
        sequence: blake(b"ZTxIdSequencHash", &seqs),
        outputs: blake(b"ZTxIdOutputsHash", &outputs_bytes(Chain::Zec, &p.outputs)?),
        sapling: blake(b"ZTxIdSaplingHash", &[]),
        orchard: blake(b"ZTxIdOrchardHash", &[]),
    })
}

fn zec_root(branch_id: u32, d: &ZecDigests, transparent: &[u8; 32]) -> [u8; 32] {
    let mut personal = *b"ZcashTxHash_\0\0\0\0";
    personal[12..].copy_from_slice(&branch_id.to_le_bytes());
    blake(&personal, &[&d.header[..], transparent, &d.sapling, &d.orchard].concat())
}

fn sighashes(p: &Plan) -> Result<Vec<String>, String> {
    let chain = Chain::parse(&p.chain)?;
    let from = script(chain, &p.from)?;
    if from.len() != 25 || from[0] != 0x76 {
        return Err("sending address must be P2PKH".into());
    }
    match chain {
        Chain::Doge => {
            // Legacy digest: the whole transaction with this input's
            // scriptSig set to the spent script and the others empty.
            let outs = outputs_bytes(chain, &p.outputs)?;
            (0..p.inputs.len())
                .map(|n| {
                    let mut t = 1u32.to_le_bytes().to_vec();
                    varint(&mut t, p.inputs.len());
                    for (j, i) in p.inputs.iter().enumerate() {
                        t.extend(txid_bytes(&i.txid)?);
                        t.extend(i.vout.to_le_bytes());
                        if j == n {
                            varint(&mut t, from.len());
                            t.extend(&from);
                        } else {
                            t.push(0);
                        }
                        t.extend(SEQUENCE.to_le_bytes());
                    }
                    varint(&mut t, p.outputs.len());
                    t.extend(&outs);
                    t.extend(0u32.to_le_bytes());
                    t.extend(1u32.to_le_bytes());
                    Ok(hex::encode(sha256d(&t)))
                })
                .collect()
        }
        Chain::Bch => {
            let mut prevouts = vec![];
            let mut seqs = vec![];
            for i in &p.inputs {
                prevouts.extend(txid_bytes(&i.txid)?);
                prevouts.extend(i.vout.to_le_bytes());
                seqs.extend(SEQUENCE.to_le_bytes());
            }
            let (hp, hs, ho) = (sha256d(&prevouts), sha256d(&seqs), sha256d(&outputs_bytes(chain, &p.outputs)?));
            p.inputs
                .iter()
                .map(|i| {
                    let mut pre = 2u32.to_le_bytes().to_vec();
                    pre.extend(hp);
                    pre.extend(hs);
                    pre.extend(txid_bytes(&i.txid)?);
                    pre.extend(i.vout.to_le_bytes());
                    varint(&mut pre, from.len());
                    pre.extend(&from);
                    pre.extend(i.value.to_le_bytes());
                    pre.extend(SEQUENCE.to_le_bytes());
                    pre.extend(ho);
                    pre.extend(0u32.to_le_bytes());
                    pre.extend(0x41u32.to_le_bytes()); // SIGHASH_ALL | FORKID, fork id 0
                    Ok(hex::encode(sha256d(&pre)))
                })
                .collect()
        }
        Chain::Zec => {
            if p.branch_id == 0 || p.expiry_height == 0 {
                return Err("Zcash needs a branch ID and expiry height".into());
            }
            let d = zec_digests(p)?;
            let mut amounts = vec![];
            let mut scripts = vec![];
            for i in &p.inputs {
                amounts.extend((i.value as i64).to_le_bytes());
                varint(&mut scripts, from.len());
                scripts.extend(&from);
            }
            let (ha, hsc) = (blake(b"ZTxTrAmountsHash", &amounts), blake(b"ZTxTrScriptsHash", &scripts));
            p.inputs
                .iter()
                .map(|i| {
                    let mut txin = txid_bytes(&i.txid)?;
                    txin.extend(i.vout.to_le_bytes());
                    txin.extend((i.value as i64).to_le_bytes());
                    varint(&mut txin, from.len());
                    txin.extend(&from);
                    txin.extend(SEQUENCE.to_le_bytes());
                    let txin = blake(b"Zcash___TxInHash", &txin);
                    let t = blake(b"ZTxIdTranspaHash", &[&[1u8][..], &d.prevouts, &ha, &hsc, &d.sequence, &d.outputs, &txin].concat());
                    Ok(hex::encode(zec_root(p.branch_id, &d, &t)))
                })
                .collect()
        }
    }
}

/// Plans a send on Dogecoin, Bitcoin Cash or Zcash: coins largest first
/// (at most 6), change back to `from` unless it's dust. `fee_rate` is per
/// byte (Dogecoin, Bitcoin Cash); Zcash's fee is ZIP-317's, 5,000 zatoshis
/// per logical action with a minimum of two. `branch_id` and
/// `expiry_height` are for Zcash only (0 otherwise).
#[wasm_bindgen]
pub fn legacy_plan(chain: &str, from: &str, to: &str, amount: &str, utxos_json: &str, fee_rate: u32, branch_id: u32, expiry_height: u32) -> Result<String, String> {
    let c = Chain::parse(chain)?;
    let amount: u64 = amount.parse().map_err(err)?;
    if amount < c.dust() {
        return Err("amount below the dust limit".into());
    }
    let to_script = script(c, to)?;
    if script(c, from)?.len() != 25 {
        return Err("sending address must be P2PKH".into());
    }
    let mut utxos: Vec<BtcInput> = serde_json::from_str(utxos_json).map_err(err)?;
    utxos.sort_by(|a, b| b.value.cmp(&a.value));
    let fee_for = |n_in: usize, change: bool| -> u64 {
        let n_out = if change { 2 } else { 1 };
        if c == Chain::Zec {
            return 5_000 * n_in.max(n_out).max(2) as u64;
        }
        // P2PKH input ~148 bytes; output 9 + script; ~10 bytes overhead.
        (10 + 148 * n_in as u64 + (9 + to_script.len() as u64) + if change { 34 } else { 0 }) * fee_rate as u64
    };
    let mut inputs = vec![];
    let mut total = 0u64;
    for u in utxos {
        if inputs.len() == 6 {
            break;
        }
        total += u.value;
        inputs.push(u);
        if total >= amount + fee_for(inputs.len(), true) {
            break;
        }
    }
    let (fee_change, fee_none) = (fee_for(inputs.len(), true), fee_for(inputs.len(), false));
    let mut outputs = vec![BtcOutput { address: to.to_string(), value: amount }];
    let fee = if total >= amount + fee_change + c.dust() {
        outputs.push(BtcOutput { address: from.to_string(), value: total - amount - fee_change });
        fee_change
    } else if total >= amount + fee_none {
        total - amount
    } else {
        return Err(format!("not enough {} for this amount and fee", c.sym()));
    };
    let (branch_id, expiry_height) = if c == Chain::Zec { (branch_id, expiry_height) } else { (0, 0) };
    let mut p = Plan { chain: c.id().into(), from: from.into(), inputs, outputs, fee, branch_id, expiry_height, sighashes: vec![] };
    p.sighashes = sighashes(&p)?;
    serde_json::to_string(&p).map_err(err)
}

fn checked(plan_json: &str) -> Result<Plan, String> {
    let p: Plan = serde_json::from_str(plan_json).map_err(err)?;
    if sighashes(&p)? != p.sighashes {
        return Err("signing hashes don't match the transaction".into());
    }
    let total_in: u64 = p.inputs.iter().map(|i| i.value).sum();
    let total_out: u64 = p.outputs.iter().map(|o| o.value).sum();
    if total_in < total_out || total_in - total_out != p.fee {
        return Err("fee doesn't add up".into());
    }
    Ok(p)
}

/// For Confirm: what a plan pays, with the signing hashes recomputed.
#[wasm_bindgen]
pub fn describe_legacy_plan(plan_json: &str) -> Result<String, String> {
    let p = checked(plan_json)?;
    let pays: Vec<_> = p.outputs.iter().filter(|o| o.address != p.from).map(|o| serde_json::json!({ "to": o.address, "sat": o.value.to_string() })).collect();
    serde_json::to_string(&serde_json::json!({ "chain": p.chain, "pays": pays, "fee_sat": p.fee.to_string(), "inputs": p.inputs.len(), "sighashes": p.sighashes })).map_err(err)
}

/// Matches each MPC signature to the input whose hash it signs and checks
/// it recovers to the key behind `pkh`: `(low-S DER, compressed key)`.
pub(crate) fn match_signatures(sighashes: &[String], pkh: &[u8], mpc_signatures_json: &str) -> Result<Vec<(Vec<u8>, Vec<u8>)>, String> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
    let sigs: Vec<serde_json::Value> = serde_json::from_str(mpc_signatures_json).map_err(err)?;
    let mut out: Vec<Option<(Vec<u8>, Vec<u8>)>> = vec![None; sighashes.len()];
    for sig in &sigs {
        let big_r = hex::decode(sig["big_r"]["affine_point"].as_str().ok_or("big_r")?).map_err(err)?;
        let s = hex::decode(sig["s"]["scalar"].as_str().ok_or("s")?).map_err(err)?;
        let recid = sig["recovery_id"].as_u64().ok_or("recovery_id")? as u8;
        if big_r.len() != 33 || s.len() != 32 || recid > 1 {
            return Err("bad signature shape".into());
        }
        let signature = Signature::from_slice(&[&big_r[1..], &s[..]].concat()).map_err(err)?;
        for (i, h) in sighashes.iter().enumerate() {
            let hash = hex32(h)?;
            let Ok(key) = VerifyingKey::recover_from_prehash(&hash, &signature, RecoveryId::from_byte(recid).unwrap()) else { continue };
            let pubkey = key.to_sec1_point(true).as_bytes().to_vec();
            if hash160(&pubkey)[..] == *pkh && out[i].is_none() {
                out[i] = Some((signature.normalize_s().to_der().as_bytes().to_vec(), pubkey));
                break;
            }
        }
    }
    out.into_iter().enumerate().map(|(i, w)| w.ok_or(format!("no valid signature for input {i}"))).collect()
}

/// The signed transaction (hex) and its txid, after every signature is
/// checked against the account's key.
#[wasm_bindgen]
pub fn legacy_signed_tx(plan_json: &str, mpc_signatures_json: &str) -> Result<String, String> {
    let p = checked(plan_json)?;
    let c = Chain::parse(&p.chain)?;
    let from = script(c, &p.from)?;
    let sigs = match_signatures(&p.sighashes, &from[3..23], mpc_signatures_json)?;
    let mut vin = vec![];
    varint(&mut vin, p.inputs.len());
    for (i, (der, pk)) in p.inputs.iter().zip(&sigs) {
        vin.extend(txid_bytes(&i.txid)?);
        vin.extend(i.vout.to_le_bytes());
        let mut ss = vec![(der.len() + 1) as u8];
        ss.extend(der);
        ss.push(c.hash_type());
        ss.push(pk.len() as u8);
        ss.extend(pk);
        varint(&mut vin, ss.len());
        vin.extend(ss);
        vin.extend(SEQUENCE.to_le_bytes());
    }
    let mut vout = vec![];
    varint(&mut vout, p.outputs.len());
    vout.extend(outputs_bytes(c, &p.outputs)?);
    let (raw, mut txid) = match c {
        Chain::Zec => {
            let mut t = vec![];
            for v in [ZEC_V5_HEADER, ZEC_V5_GROUP, p.branch_id, 0, p.expiry_height] {
                t.extend(v.to_le_bytes());
            }
            t.extend(&vin);
            t.extend(&vout);
            t.extend([0, 0, 0]); // no Sapling spends or outputs, no Orchard actions
            let d = zec_digests(&p)?;
            let transparent = blake(b"ZTxIdTranspaHash", &[d.prevouts, d.sequence, d.outputs].concat());
            (t, zec_root(p.branch_id, &d, &transparent))
        }
        _ => {
            let version: u32 = if c == Chain::Doge { 1 } else { 2 };
            let mut t = version.to_le_bytes().to_vec();
            t.extend(&vin);
            t.extend(&vout);
            t.extend(0u32.to_le_bytes());
            let id = sha256d(&t);
            (t, id)
        }
    };
    txid.reverse();
    serde_json::to_string(&serde_json::json!({ "raw": hex::encode(raw), "txid": hex::encode(txid) })).map_err(err)
}

/// The output script (hex) for a testnet address on any Bitcoin-family
/// chain here (SegWit too, for Bitcoin and Litecoin): what agent payment
/// grants and requests carry.
#[wasm_bindgen]
pub fn utxo_script(chain: &str, address: &str) -> Result<String, String> {
    match chain {
        "bitcoin" => Ok(hex::encode(super::btc_script_pub(address, "tb")?)),
        "litecoin" => Ok(hex::encode(super::btc_script_pub(address, "tltc")?)),
        _ => legacy_script_pubkey(chain, address),
    }
}

/// The testnet address an output script pays, for showing agent payment
/// grants and requests in words. Refuses scripts that aren't standard.
#[wasm_bindgen]
pub fn utxo_address(chain: &str, script_hex: &str) -> Result<String, String> {
    let s = hex::decode(script_hex).map_err(err)?;
    let bad = || "not a standard output script".to_string();
    let p2pkh = s.len() == 25 && s[..3] == [0x76, 0xa9, 0x14] && s[23..] == [0x88, 0xac];
    let p2sh = s.len() == 23 && s[..2] == [0xa9, 0x14] && s[22] == 0x87;
    match chain {
        "bitcoin" | "litecoin" => {
            let hrp = bech32::Hrp::parse(if chain == "bitcoin" { "tb" } else { "tltc" }).unwrap();
            if s.len() >= 4 && s.len() <= 42 && s[1] as usize == s.len() - 2 && (s[0] == 0 || (0x51..=0x60).contains(&s[0])) {
                let v = if s[0] == 0 { 0 } else { s[0] - 0x50 };
                let fe = bech32::Fe32::try_from(v).map_err(|_| bad())?;
                return bech32::segwit::encode(hrp, fe, &s[2..]).map_err(|_| bad());
            }
            if p2pkh {
                return Ok(bs58::encode([&[0x6f][..], &s[3..23]].concat()).with_check().into_string());
            }
            if p2sh {
                let v = if chain == "litecoin" { 0x3a } else { 0xc4 };
                return Ok(bs58::encode([&[v][..], &s[2..22]].concat()).with_check().into_string());
            }
            Err(bad())
        }
        "bitcoin-cash" => {
            if p2pkh {
                return Ok(super::cashaddr_typed("bchtest", 0, &s[3..23].try_into().unwrap()));
            }
            if p2sh {
                return Ok(super::cashaddr_typed("bchtest", 8, &s[2..22].try_into().unwrap()));
            }
            Err(bad())
        }
        "zcash" => {
            if p2pkh {
                return Ok(bs58::encode([&[0x1d, 0x25][..], &s[3..23]].concat()).with_check().into_string());
            }
            if p2sh {
                return Ok(bs58::encode([&[0x1c, 0xba][..], &s[2..22]].concat()).with_check().into_string());
            }
            Err(bad())
        }
        _ => Err(format!("{chain} isn't offered to agents")),
    }
}

/// The output script (hex) for a testnet address on Dogecoin, Bitcoin Cash
/// or Zcash; the relayer derives Electrum script hashes from it.
#[wasm_bindgen]
pub fn legacy_script_pubkey(chain: &str, address: &str) -> Result<String, String> {
    Ok(hex::encode(script(Chain::parse(chain)?, address)?))
}
