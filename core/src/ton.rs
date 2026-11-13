//! TON: the account's wallet (v4r2, workchain 0) and the owner's sends.
//!
//! The address is the hash of the wallet's state init (the standard v4r2
//! code and a data cell holding seqno 0, subwallet 698983191 and the MPC
//! key), so the wallet deploys itself with its first outgoing message.
//! The key signs the hash of the signing message (ton_cell.rs).

use wasm_bindgen::prelude::*;

use crate::ton_cell::{self, Addr, Cell};
use crate::err;

/// Wallet v4r2's code, as @ton/ton 15.4 ships it; its hash is the well-known
/// feb5ff68…bd5c0, checked in the tests.
pub const V4R2_CODE: &str = "b5ee9c72010214010002d4000114ff00f4a413f4bcf2c80b01020120020f020148030602e6d001d0d3032171b0925f04e022d749c120925f04e002d31f218210706c7567bd22821064737472bdb0925f05e003fa403020fa4401c8ca07cbffc9d0ed44d0810140d721f404305c810108f40a6fa131b3925f07e005d33fc8258210706c7567ba923830e30d03821064737472ba925f06e30d0405007801fa00f40430f8276f2230500aa121bef2e0508210706c7567831eb17080185004cb0526cf1658fa0219f400cb6917cb1f5260cb3f20c98040fb0006008a5004810108f45930ed44d0810140d720c801cf16f400c9ed540172b08e23821064737472831eb17080185005cb055003cf1623fa0213cb6acb1fcb3fc98040fb00925f03e2020120070e020120080d020158090a003db29dfb513420405035c87d010c00b23281f2fff274006040423d029be84c600201200b0c0019adce76a26840206b90eb85ffc00019af1df6a26840106b90eb858fc00011b8c97ed44d0d70b1f80059bd242b6f6a2684080a06b90fa0218470d4080847a4937d29910ce6903e9ff9837812801b7810148987159f318404f8f28308d71820d31fd31fd31f02f823bbf264ed44d0d31fd31fd3fff404d15143baf2a15151baf2a205f901541064f910f2a3f80024a4c8cb1f5240cb1f5230cbff5210f400c9ed54f80f01d30721c0009f6c519320d74a96d307d402fb00e830e021c001e30021c002e30001c0039130e30d03a4c8cb1f12cb1fcbff10111213006ed207fa00d4d422f90005c8ca0715cbffc9d077748018c8cb05cb0222cf165005fa0214cb6b12ccccc973fb00c84014810108f451f2a7020070810108d718fa00d33fc8542047810108f451f2a782106e6f746570748018c8cb05cb025006cf165004fa0214cb6a12cb1fcb3fc973fb0002006c810108d718fa00d33f305224810108f459f2a782106473747270748018c8cb05cb025005cf165003fa0213cb6acb1f12cb3fc973fb00000af400c9ed54";

/// The wallet's state init: split_depth and special absent, code and data
/// present, no library.
pub fn state_init(pk: &[u8; 32]) -> Result<Cell, String> {
    let code = Cell::from_boc(&hex::decode(V4R2_CODE).map_err(err)?)?;
    let data = Cell::new().uint(0, 32).uint(ton_cell::V4_SUBWALLET as u128, 32).bytes(pk).bit(false).done()?;
    Ok(Cell::new().bit(false).bit(false).bit(true).push_ref(code).bit(true).push_ref(data).bit(false).done()?)
}

pub fn wallet(pk: &[u8; 32]) -> Result<Addr, String> {
    Ok(Addr { wc: 0, hash: state_init(pk)?.hash() })
}

/// The account's address as wallets show an undeployed one: testnet,
/// non-bounceable (0Q…).
pub fn address(pk: &[u8; 32]) -> Result<String, String> {
    Ok(ton_cell::friendly(&wallet(pk)?, false, true))
}

fn key(derived_public_key: &str) -> Result<[u8; 32], String> {
    let b = derived_public_key.strip_prefix("ed25519:").ok_or("expected an ed25519 key")?;
    bs58::decode(b).into_vec().map_err(err)?.try_into().map_err(|_| "expected 32 bytes".to_string())
}

/// One TON payment from the owner: the signing message (BOC hex) and its
/// hash, which the MPC key signs. Bounce follows the recipient's address
/// form (EQ/kQ bounceable, UQ/0Q not), as wallets do.
#[wasm_bindgen]
pub fn ton_transfer(derived_public_key: &str, to: &str, nanotons: &str, seqno: u32, valid_until: u32, comment: &str) -> Result<String, String> {
    let me = wallet(&key(derived_public_key)?)?;
    let (to, bounce) = ton_cell::parse_address(to)?;
    if to == me {
        return Err("sending to yourself".into());
    }
    let amount: u128 = nanotons.parse().map_err(err)?;
    if amount == 0 {
        return Err("amount must be more than zero".into());
    }
    let body = if comment.is_empty() { None } else { Some(ton_cell::comment(comment)?) };
    let m = ton_cell::v4_signing_message(seqno, valid_until, vec![ton_cell::internal_message(&to, amount, bounce, body)?])?;
    Ok(serde_json::json!({ "signing_boc": hex::encode(m.boc()), "signing_hash": hex::encode(m.hash()) }).to_string())
}

/// Decodes a signing message for Confirm: only the exact one-payment layout
/// ton_transfer builds is recognised, by rebuilding it from its fields.
#[wasm_bindgen]
pub fn describe_ton(signing_boc_hex: &str) -> Result<String, String> {
    let c = Cell::from_boc(&hex::decode(signing_boc_hex).map_err(err)?)?;
    let not = || "not a plain TON payment".to_string();
    let rd = |bits: &[bool], from: usize, n: usize| -> u128 { bits[from..from + n].iter().fold(0u128, |a, b| a << 1 | *b as u128) };
    if c.bits.len() != 32 * 3 + 16 || c.refs.len() != 1 || rd(&c.bits, 0, 32) != ton_cell::V4_SUBWALLET as u128 || rd(&c.bits, 96, 16) != ton_cell::SEND_MODE as u128 {
        return Err(not());
    }
    let (valid_until, seqno) = (rd(&c.bits, 32, 32) as u32, rd(&c.bits, 64, 32) as u32);
    let m = &c.refs[0];
    let b = &m.bits;
    // int_msg_info 0, ihr_disabled 1, bounce, bounced 0, src 00, dest 100 wc8 hash256.
    if b.len() < 4 + 2 + 267 + 4 || b[0] || !b[1] || b[3] || rd(b, 4, 5) != 0b00100 || rd(b, 9, 8) != 0 {
        return Err(not());
    }
    let bounce = b[2];
    let hash: [u8; 32] = (0..32).map(|i| rd(b, 17 + 8 * i, 8) as u8).collect::<Vec<_>>().try_into().unwrap();
    let to = Addr { wc: 0, hash };
    let len = rd(b, 273, 4) as usize;
    if len > 16 || b.len() < 277 + len * 8 {
        return Err(not());
    }
    let amount = rd(b, 277, len * 8);
    let comment = match m.refs.first() {
        None => String::new(),
        Some(body) => {
            if body.bits.len() < 32 || body.bits.len() % 8 != 0 || rd(&body.bits, 0, 32) != 0 || !body.refs.is_empty() {
                return Err(not());
            }
            let bytes: Vec<u8> = (4..body.bits.len() / 8).map(|i| rd(&body.bits, 8 * i, 8) as u8).collect();
            String::from_utf8(bytes).map_err(|_| not())?
        }
    };
    // Rebuild: anything the decoder skipped would change the hash.
    let body = if comment.is_empty() { None } else { Some(ton_cell::comment(&comment)?) };
    let again = ton_cell::v4_signing_message(seqno, valid_until, vec![ton_cell::internal_message(&to, amount, bounce, body)?])?;
    if again != c {
        return Err(not());
    }
    Ok(serde_json::json!({
        "kind": "ton_transfer", "to": ton_cell::friendly(&to, bounce, true), "nanotons": amount.to_string(),
        "comment": comment, "seqno": seqno, "valid_until": valid_until, "signing_hash": hex::encode(c.hash()),
    })
    .to_string())
}

/// The external message (BOC, base64, for sendBoc) carrying a signed
/// signing message, after checking the MPC signature verifies for the
/// account's key over its hash. `deploy` adds the state init (seqno 0).
#[wasm_bindgen]
pub fn ton_signed_tx(signing_boc_hex: &str, mpc_signature_json: &str, derived_public_key: &str, deploy: bool) -> Result<String, String> {
    use base64::Engine;
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let pk = key(derived_public_key)?;
    let m = Cell::from_boc(&hex::decode(signing_boc_hex).map_err(err)?)?;
    let sig: serde_json::Value = serde_json::from_str(mpc_signature_json).map_err(err)?;
    let bytes: Vec<u8> = sig["signature"].as_array().ok_or("signature")?.iter().map(|v| v.as_u64().unwrap_or(256) as u8).collect();
    let bytes: [u8; 64] = bytes.try_into().map_err(|_| "signature must be 64 bytes".to_string())?;
    VerifyingKey::from_bytes(&pk).map_err(err)?.verify(&m.hash(), &Signature::from_bytes(&bytes)).map_err(|_| "signature is not from this account".to_string())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(external(&pk, &bytes, &m, deploy)?.boc()))
}

/// ext_in_msg_info$10, src none, dest the wallet, import fee 0; the state
/// init (in a reference) when deploying; the body (signature then the
/// signing message) in a reference.
pub fn external(pk: &[u8; 32], sig: &[u8; 64], signing: &Cell, deploy: bool) -> Result<Cell, String> {
    let mut body = Cell::new();
    body.bytes(sig);
    body.bits.extend(&signing.bits);
    body.refs = signing.refs.clone();
    let body = body.done()?;
    let mut c = Cell::new();
    c.uint(0b10, 2).addr(None).addr(Some(&wallet(pk)?)).coins(0);
    if deploy {
        c.bit(true).bit(true).push_ref(state_init(pk)?);
    } else {
        c.bit(false);
    }
    c.bit(true).push_ref(body);
    Ok(c.done()?)
}
