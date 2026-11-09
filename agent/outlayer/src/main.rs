//! The Marvii demo agent's session key, held inside an OutLayer TEE.
//!
//! OutLayer derives the key inside an Intel TDX enclave from this exact build
//! (`bind: "wasm"`) and the NEAR account that pays for the run. Nobody reads
//! it: not the operator, not OutLayer, not us. A new build gets a new key, so
//! the key names the code, and OutLayer's per-run TDX quote names the build's
//! SHA-256.
//!
//! The module never signs bytes it's handed. It builds the agent's message
//! itself, in the extension's format (`marvii-session-v1`), from fields it has
//! parsed and checked, and only for Hyperliquid actions. The extension on NEAR
//! still checks every rule and cap; this only decides where the key lives.
//!
//! Input (stdin, JSON):
//!   {"operation": "public_key"}
//!   {"operation": "sign", "extension": "…", "wallet": "…", "key_id": "…",
//!    "nonce": 7, "action": {"type": "hl_order", …}}
//! Output (stdout, JSON): {"public_key": hex} or
//!   {"public_key", "msg", "signature", "signer"}; on failure {"error": "…"}.
//! `sign` runs only from a NEAR transaction: the deadline comes from the
//! block's timestamp, never from the input.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Read;

/// The extension's domain tag.
const DOMAIN: &str = "marvii-session-v1";
/// The key's path in the manifest.
const KEY: &str = "marvii-agent";
/// How long a signed message stays valid. The extension accepts at most 10
/// minutes ahead.
const VALID_MS: u64 = 5 * 60 * 1000;

#[cfg(target_arch = "wasm32")]
#[used]
#[link_section = "outlayer.manifest"]
static OUTLAYER_MANIFEST: [u8; include_bytes!("../manifest.json").len()] = *include_bytes!("../manifest.json");

#[cfg(target_arch = "wasm32")]
mod host {
    wit_bindgen::generate!({ world: "signing-keys-host", path: "wit" });
}

/// The Hyperliquid actions this agent may ask for, in the extension's JSON
/// shape (`rules::Action`). Unknown types and fields are refused.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    HlOrder { asset: u32, is_buy: bool, price: String, size: String, reduce_only: bool, tif: String, nonce: u64 },
    HlCancel { asset: u32, oid: u64, nonce: u64 },
    HlLeverage { asset: u32, is_cross: bool, leverage: u32, nonce: u64 },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    operation: String,
    extension: Option<String>,
    wallet: Option<String>,
    key_id: Option<String>,
    nonce: Option<u64>,
    action: Option<Action>,
}

/// The message the extension verifies, field for field (`SessionMessage`).
#[derive(Serialize)]
struct SessionMessage<'a> {
    domain: &'a str,
    extension: &'a str,
    wallet: &'a str,
    key_id: &'a str,
    nonce: u64,
    deadline_ms: u64,
    action: &'a Action,
}

/// A NEAR account ID as NEAR defines it: 2 to 64 characters of lowercase
/// letters, digits and `-_.`, no separator at either end or twice in a row.
fn account_ok(a: &str) -> bool {
    let b = a.as_bytes();
    (2..=64).contains(&b.len())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"-_.".contains(c))
        && !b"-_.".contains(&b[0])
        && !b"-_.".contains(&b[b.len() - 1])
        && !b.windows(2).any(|w| b"-_.".contains(&w[0]) && b"-_.".contains(&w[1]))
}

fn decimal_ok(s: &str) -> bool {
    let mut parts = s.split('.');
    let int = parts.next().unwrap_or("");
    let frac = parts.next();
    parts.next().is_none()
        && !int.is_empty()
        && int.len() <= 12
        && int.bytes().all(|c| c.is_ascii_digit())
        && frac.map_or(true, |f| !f.is_empty() && f.len() <= 8 && f.bytes().all(|c| c.is_ascii_digit()))
}

fn check_action(a: &Action) -> Result<(), String> {
    match a {
        Action::HlOrder { price, size, tif, .. } => {
            if !decimal_ok(price) || !decimal_ok(size) {
                return Err("price and size must be plain decimals".into());
            }
            if !["Gtc", "Ioc", "Alo"].contains(&tif.as_str()) {
                return Err("tif must be Gtc, Ioc or Alo".into());
            }
        }
        Action::HlLeverage { leverage, .. } if *leverage == 0 || *leverage > 50 => return Err("leverage must be 1 to 50".into()),
        _ => {}
    }
    Ok(())
}

/// Builds the exact message text the key signs, and its SHA-256 (what the
/// extension verifies the signature over).
pub fn build_message(extension: &str, wallet: &str, key_id: &str, nonce: u64, block_ms: u64, action: &Action) -> Result<(String, [u8; 32]), String> {
    if !account_ok(extension) || !account_ok(wallet) {
        return Err("extension and wallet must be NEAR account IDs".into());
    }
    if key_id.is_empty() || key_id.len() > 64 || !key_id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_') {
        return Err("key_id must be 1 to 64 letters, digits, - or _".into());
    }
    check_action(action)?;
    let msg = serde_json::to_string(&SessionMessage { domain: DOMAIN, extension, wallet, key_id, nonce, deadline_ms: block_ms + VALID_MS, action })
        .map_err(|e| e.to_string())?;
    Ok((msg.clone(), Sha256::digest(msg.as_bytes()).into()))
}

#[cfg(target_arch = "wasm32")]
fn run(raw: &str) -> Result<Value, String> {
    use host::outlayer::signing_keys::api;
    let input: Input = serde_json::from_str(raw).map_err(|e| format!("bad input: {e}"))?;
    let public_key = hex::encode(api::public_key(KEY, None)?);
    match input.operation.as_str() {
        "public_key" => Ok(json!({ "public_key": public_key })),
        "sign" => {
            // The block's time, set by NEAR; empty on an HTTPS call.
            let block_ns: u64 = std::env::var("NEAR_BLOCK_TIMESTAMP").ok().and_then(|t| t.parse().ok()).filter(|t| *t > 0)
                .ok_or("sign runs only from a NEAR transaction")?;
            let need = |v: Option<String>, n: &str| v.ok_or(format!("missing {n}"));
            let action = input.action.ok_or("missing action")?;
            let (msg, digest) = build_message(
                &need(input.extension, "extension")?,
                &need(input.wallet, "wallet")?,
                &need(input.key_id, "key_id")?,
                input.nonce.ok_or("missing nonce")?,
                block_ns / 1_000_000,
                &action,
            )?;
            let signature = hex::encode(api::sign(KEY, None, &digest)?);
            Ok(json!({ "public_key": public_key, "msg": msg, "signature": signature, "signer": std::env::var("NEAR_USER_ACCOUNT_ID").unwrap_or_default() }))
        }
        _ => Err("operation must be public_key or sign".into()),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run(_raw: &str) -> Result<Value, String> {
    Err("build for wasm32-wasip2".into())
}

fn main() {
    let mut raw = String::new();
    let out = match std::io::stdin().read_to_string(&mut raw) {
        Ok(_) => run(&raw).unwrap_or_else(|e| json!({ "error": e })),
        Err(e) => json!({ "error": e.to_string() }),
    };
    print!("{out}");
}

#[cfg(test)]
mod tests;
