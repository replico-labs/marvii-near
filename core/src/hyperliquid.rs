//! The owner moves USDC between Hyperliquid's spot and perps balances
//! (`usdClassTransfer`), signed by the account's EVM key through Chain
//! Signatures after a passkey tap.
//!
//! A Hyperliquid "user-signed" action: EIP-712 in the
//! `HyperliquidSignTransaction` domain. The digest is built here from the
//! fields, with `hyperliquidChain` fixed to `Testnet`, so the signature is
//! good only for this transfer on Hyperliquid testnet.

use super::err;
use sha3::{Digest, Keccak256};
use wasm_bindgen::prelude::*;

/// The chain ID Hyperliquid's own SDK signs with (0x66eee); any value is
/// accepted, the network is bound by `hyperliquidChain`.
pub const HL_SIGNATURE_CHAIN_ID: u64 = 0x66eee;
const HL_CHAIN: &str = "Testnet";

fn keccak(b: &[u8]) -> [u8; 32] {
    Keccak256::digest(b).into()
}

fn word_u64(v: u64) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[24..].copy_from_slice(&v.to_be_bytes());
    w
}

/// A positive plain decimal with at most 6 decimals (USDC), e.g. "49.5".
fn check_amount(a: &str) -> Result<(), String> {
    let (i, f) = a.split_once('.').unwrap_or((a, ""));
    let ok = !i.is_empty()
        && i.bytes().all(|b| b.is_ascii_digit())
        && f.bytes().all(|b| b.is_ascii_digit())
        && f.len() <= 6
        && !(a.contains('.') && (f.is_empty() || f.ends_with('0')))
        && !(i.len() > 1 && i.starts_with('0'))
        && a.bytes().any(|b| (b'1'..=b'9').contains(&b));
    if ok { Ok(()) } else { Err("amount must be a plain number of USDC, e.g. 49.5".into()) }
}

/// The EIP-712 digest of a spot↔perps USDC transfer on Hyperliquid testnet.
#[wasm_bindgen]
pub fn hl_transfer_hash(amount: &str, to_perp: bool, nonce: u64) -> Result<String, String> {
    check_amount(amount)?;
    let mut domain = keccak(b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)").to_vec();
    domain.extend(keccak(b"HyperliquidSignTransaction"));
    domain.extend(keccak(b"1"));
    domain.extend(word_u64(HL_SIGNATURE_CHAIN_ID));
    domain.extend([0u8; 32]);
    let mut s = keccak(b"HyperliquidTransaction:UsdClassTransfer(string hyperliquidChain,string amount,bool toPerp,uint64 nonce)").to_vec();
    s.extend(keccak(HL_CHAIN.as_bytes()));
    s.extend(keccak(amount.as_bytes()));
    s.extend(word_u64(to_perp as u64));
    s.extend(word_u64(nonce));
    let mut m = vec![0x19, 0x01];
    m.extend(keccak(&domain));
    m.extend(keccak(&s));
    Ok(hex::encode(keccak(&m)))
}

/// The passkey's request: the wallet asks the MPC signer to sign that
/// digest with its EVM key (path `evm`).
#[wasm_bindgen]
pub fn owner_hl_transfer_message(chain_id: &str, wallet: &str, nonce: u32, created_at_secs: i64, signer: &str, amount: &str, to_perp: bool, hl_nonce: u64) -> Result<String, String> {
    let h = hl_transfer_hash(amount, to_perp, hl_nonce)?;
    let args = serde_json::json!({ "request": { "path": "evm", "payload_v2": { "Ecdsa": h }, "domain_id": 0 } });
    super::call_message(chain_id, wallet, nonce, created_at_secs, signer, "sign", &args.to_string(), "1", 30).map_err(err)
}

/// The exact action Hyperliquid receives, for the page to send.
#[wasm_bindgen]
pub fn hl_transfer_action(amount: &str, to_perp: bool, nonce: u64) -> Result<String, String> {
    check_amount(amount)?;
    Ok(serde_json::json!({
        "type": "usdClassTransfer", "hyperliquidChain": HL_CHAIN, "signatureChainId": format!("0x{HL_SIGNATURE_CHAIN_ID:x}"),
        "amount": amount, "toPerp": to_perp, "nonce": nonce,
    })
    .to_string())
}
