//! The wallet's own types, compiled to WebAssembly for the page.
//!
//! The page must never rebuild these by hand: a wrong byte in the hash
//! means the passkey signs something other than what the user saw. So the
//! hash, the account ID and the proof all come from `defuse-wallet` itself.

use defuse_crypto::p256::{P256CompressedPublicKey, P256Signature};
use defuse_wallet::{GlobalContractId, RequestMessage, State, StateInit, StateInitV1};
use defuse_wallet_webauthn::{WalletWebauthnProof, webauthn::WebauthnAssertion};
use p256::ecdsa::{Signature, VerifyingKey};
use wasm_bindgen::prelude::*;

fn err(e: impl core::fmt::Display) -> String {
    e.to_string()
}

fn hex32(s: &str) -> Result<[u8; 32], String> {
    hex::decode(s.trim_start_matches("0x"))
        .map_err(err)?
        .try_into()
        .map_err(|_| err("expected 32 bytes"))
}

/// Compresses a SEC1 P-256 public key (65 bytes uncompressed or 33
/// compressed), as hex. Checks it's a point on the curve.
#[wasm_bindgen]
pub fn compress_public_key(sec1_hex: &str) -> Result<String, String> {
    let bytes = hex::decode(sec1_hex).map_err(err)?;
    let key = VerifyingKey::from_sec1_bytes(&bytes).map_err(|_| err("not a P-256 public key"))?;
    Ok(hex::encode(P256CompressedPublicKey::from(&key).0))
}

pub fn state_init(code_hash_hex: &str, public_key_hex: &str, extension: &str) -> Result<StateInit, String> {
    let pk: [u8; 33] = hex::decode(compress_public_key(public_key_hex)?)
        .map_err(err)?
        .try_into()
        .map_err(|_| err("bad public key"))?;
    let extension = extension.parse::<defuse_wallet::AccountId>().map_err(err)?;
    // Same as WalletBuilder::new().extension(..).build(..) in the SDK.
    Ok(StateInit::V1(StateInitV1 {
        code: GlobalContractId::CodeHash(hex32(code_hash_hex)?),
        data: State::new(P256CompressedPublicKey(pk)).extensions([extension]).as_storage(),
    }))
}

/// The wallet's account ID (`0s…`), known before it exists on-chain.
#[wasm_bindgen]
pub fn wallet_account_id(code_hash_hex: &str, public_key_hex: &str, extension: &str) -> Result<String, String> {
    Ok(state_init(code_hash_hex, public_key_hex, extension)?.derive_account_id().to_string())
}

/// Parses a request message (the wallet's JSON form) and returns its
/// canonical JSON back, so the page shows and signs the same parsed value.
#[wasm_bindgen]
pub fn normalize_request(msg_json: &str) -> Result<String, String> {
    let msg: RequestMessage = serde_json::from_str(msg_json).map_err(err)?;
    serde_json::to_string(&msg).map_err(err)
}

/// The 32 bytes the passkey signs as its WebAuthn challenge:
/// `SHA3-256("NEAR_WALLET_CONTRACT/V1" || borsh(msg))`, as hex.
#[wasm_bindgen]
pub fn request_hash(msg_json: &str) -> Result<String, String> {
    let msg: RequestMessage = serde_json::from_str(msg_json).map_err(err)?;
    Ok(hex::encode(msg.hash()))
}

/// Builds the proof `w_execute_signed` expects from a WebAuthn assertion.
/// The DER signature is converted to r||s and normalised to low S, which
/// the wallet requires.
#[wasm_bindgen]
pub fn make_proof(authenticator_data_hex: &str, client_data_json: &str, der_signature_hex: &str) -> Result<String, String> {
    let der = hex::decode(der_signature_hex).map_err(err)?;
    let sig = Signature::from_der(&der).map_err(|_| err("bad DER signature"))?;
    let sig = sig.normalize_s();
    let proof = WalletWebauthnProof::<P256Signature> {
        assertion: WebauthnAssertion {
            authenticator_data: hex::decode(authenticator_data_hex).map_err(err)?,
            client_data_json: client_data_json.to_string(),
        },
        signature: P256Signature(sig.to_bytes().into()),
    };
    serde_json::to_string(&proof).map_err(err)
}

/// Checks a proof against a public key and message, with the wallet's own
/// verifier. The page runs this before sending, so a bad proof never
/// reaches the relayer.
#[wasm_bindgen]
pub fn verify_proof(public_key_hex: &str, msg_json: &str, proof: &str) -> Result<bool, String> {
    use defuse_wallet::SignatureSchema;
    use defuse_wallet_webauthn::{WalletWebauthn, p256::P256, webauthn::IgnoreUserVerification};
    let pk: [u8; 33] = hex::decode(compress_public_key(public_key_hex)?)
        .map_err(err)?
        .try_into()
        .map_err(|_| err("bad public key"))?;
    let msg: RequestMessage = serde_json::from_str(msg_json).map_err(err)?;
    Ok(WalletWebauthn::<P256, IgnoreUserVerification>::verify_request_msg(&P256CompressedPublicKey(pk), &msg, proof))
}

/// The EVM address for an MPC derived key (`secp256k1:<base58 of x||y>`,
/// as the signer's `derived_public_key` view returns it):
/// the last 20 bytes of `keccak256(x||y)`.
#[wasm_bindgen]
pub fn evm_address(derived_public_key: &str) -> Result<String, String> {
    use sha3::{Digest, Keccak256};
    let b58 = derived_public_key.strip_prefix("secp256k1:").ok_or("expected secp256k1:")?;
    let xy = bs58::decode(b58).into_vec().map_err(err)?;
    if xy.len() != 64 {
        return Err(err("expected 64 bytes"));
    }
    let h = Keccak256::digest(&xy);
    Ok(format!("0x{}", hex::encode(&h[12..])))
}

/// Builds the message for one function call from the wallet, with the
/// wallet's own types, so the page never assembles the JSON by hand.
/// `created_at_secs` should be about a minute before now (the wallet
/// refuses messages from the future).
#[wasm_bindgen]
#[allow(clippy::too_many_arguments)]
pub fn call_message(
    chain_id: &str,
    wallet: &str,
    nonce: u32,
    created_at_secs: i64,
    receiver: &str,
    method: &str,
    args_json: &str,
    deposit_yocto: &str,
    gas_tgas: u64,
) -> Result<String, String> {
    use defuse_wallet::{Gas, NearPromise, NearToken, Request, Timestamp, actions::FunctionCall};
    let args: serde_json::Value = serde_json::from_str(args_json).map_err(err)?;
    let deposit: u128 = deposit_yocto.parse().map_err(err)?;
    let promise = NearPromise::new(receiver.parse::<defuse_wallet::AccountId>().map_err(err)?).function_call(
        FunctionCall::name(method)
            .args(serde_json::to_vec(&args).map_err(err)?)
            .attach_deposit(NearToken::from_yoctonear(deposit))
            .gas(Gas::from_tgas(gas_tgas)),
    );
    let msg = RequestMessage {
        pay_for_gas: false,
        chain_id: chain_id.to_string(),
        signer_id: wallet.parse().map_err(err)?,
        nonce,
        created_at: Timestamp::from_secs(created_at_secs).ok_or("bad time")?,
        timeout: core::time::Duration::from_secs(600),
        request: Request { internal: vec![], external: vec![promise] },
    };
    serde_json::to_string(&msg).map_err(err)
}

/// An empty request: the first one deploys the wallet (the relayer
/// attaches the state init) without doing anything else.
#[wasm_bindgen]
pub fn empty_message(chain_id: &str, wallet: &str, nonce: u32, created_at_secs: i64) -> Result<String, String> {
    use defuse_wallet::{Request, Timestamp};
    let msg = RequestMessage {
        pay_for_gas: false,
        chain_id: chain_id.to_string(),
        signer_id: wallet.parse().map_err(err)?,
        nonce,
        created_at: Timestamp::from_secs(created_at_secs).ok_or("bad time")?,
        timeout: core::time::Duration::from_secs(600),
        request: Request::default(),
    };
    serde_json::to_string(&msg).map_err(err)
}

/// Decodes a message into what the Confirm screen shows: who signs, until
/// when, every wallet op and every call with its decoded arguments. It
/// works from the parsed message, which is exactly what gets hashed.
#[wasm_bindgen]
pub fn describe_message(msg_json: &str) -> Result<String, String> {
    use defuse_wallet::{WalletOp, actions::NearAction};
    use serde_json::json;
    let msg: RequestMessage = serde_json::from_str(msg_json).map_err(err)?;
    let internal: Vec<_> = msg
        .request
        .internal
        .iter()
        .map(|op| match op {
            WalletOp::SetSignatureMode { enable } => json!({ "op": "set_signature_mode", "enable": enable }),
            WalletOp::AddExtension { account_id } => json!({ "op": "add_extension", "account_id": account_id }),
            WalletOp::RemoveExtension { account_id } => json!({ "op": "remove_extension", "account_id": account_id }),
            #[allow(unreachable_patterns)]
            other => json!({ "op": "unknown", "debug": format!("{other:?}") }),
        })
        .collect();
    let external: Vec<_> = msg
        .request
        .external
        .iter()
        .map(|p| {
            let actions: Vec<_> = p
                .actions
                .iter()
                .map(|a| match a {
                    NearAction::FunctionCall(f) => json!({
                        "kind": "function_call",
                        "method": f.function_name,
                        // Shown as text when it's JSON, else as hex.
                        "args": serde_json::from_slice::<serde_json::Value>(&f.args)
                            .unwrap_or_else(|_| json!({ "hex": hex::encode(&f.args) })),
                        "deposit_yocto": f.deposit.as_yoctonear().to_string(),
                        "gas_tgas": f.gas.as_tgas(),
                    }),
                    NearAction::Transfer(t) => json!({ "kind": "transfer", "amount_yocto": t.amount.as_yoctonear().to_string() }),
                    other => json!({ "kind": "other", "debug": format!("{other:?}") }),
                })
                .collect();
            json!({ "receiver_id": p.receiver_id, "refund_to": p.refund_to, "actions": actions })
        })
        .collect();
    serde_json::to_string(&json!({
        "chain_id": msg.chain_id,
        "signer_id": msg.signer_id,
        "nonce": msg.nonce,
        "created_at": msg.created_at.to_string(),
        "deadline": msg.deadline().to_string(),
        "internal": internal,
        "external": external,
    }))
    .map_err(err)
}

// ---- EVM transactions the owner approves ----

/// The extension's own EIP-1559 encoder and decoder: the same source file,
/// so the page computes exactly the hash the extension would.
#[path = "../../extension/src/evm.rs"]
#[allow(dead_code)]
mod evm;

/// `supply(address,uint256,address,uint16)`, from `cast sig`: Aave V3.
const SEL_AAVE_SUPPLY: [u8; 4] = [0x61, 0x7b, 0xa0, 0x37];

#[derive(serde::Deserialize)]
struct EvmTxJson {
    chain_id: u64,
    nonce: u64,
    max_priority_fee_per_gas: String,
    max_fee_per_gas: String,
    gas_limit: u64,
    to: String,
    value: String,
    data: String,
}

fn parse_tx(tx_json: &str) -> Result<evm::Eip1559, String> {
    let t: EvmTxJson = serde_json::from_str(tx_json).map_err(err)?;
    Ok(evm::Eip1559 {
        chain_id: t.chain_id,
        nonce: t.nonce,
        max_priority_fee_per_gas: t.max_priority_fee_per_gas.parse().map_err(err)?,
        max_fee_per_gas: t.max_fee_per_gas.parse().map_err(err)?,
        gas_limit: t.gas_limit,
        to: evm::parse_address(&t.to)?,
        value: t.value.parse().map_err(err)?,
        data: evm::parse_hex(&t.data)?,
    })
}

fn tx_hash(t: &evm::Eip1559) -> [u8; 32] {
    use sha3::{Digest, Keccak256};
    let mut out = vec![0x02];
    out.extend(t.rlp_payload());
    Keccak256::digest(&out).into()
}

fn word_addr(w: &[u8]) -> Result<String, String> {
    if w[..12].iter().any(|b| *b != 0) {
        return Err("address word has dirty high bytes".into());
    }
    Ok(format!("0x{}", hex::encode(&w[12..32])))
}

fn word_dec(w: &[u8]) -> Result<String, String> {
    if w[..16].iter().any(|b| *b != 0) {
        return Err("amount too large".into());
    }
    Ok(u128::from_be_bytes(w[16..32].try_into().unwrap()).to_string())
}

/// Decodes an EVM transaction for the Confirm screen and computes the hash
/// the passkey will approve. Unknown functions are shown as such, never
/// guessed.
#[wasm_bindgen]
pub fn describe_evm_tx(tx_json: &str) -> Result<String, String> {
    use serde_json::json;
    let t = parse_tx(tx_json)?;
    let call = match evm::decode(&t.data)? {
        evm::Decoded::Native => json!({ "kind": "native_transfer" }),
        evm::Decoded::Transfer { recipient, amount } => json!({ "kind": "erc20_transfer", "token": evm::fmt_address(&t.to), "recipient": evm::fmt_address(&recipient), "amount": amount.to_string() }),
        evm::Decoded::Approve { spender, amount } => json!({ "kind": "erc20_approve", "token": evm::fmt_address(&t.to), "spender": evm::fmt_address(&spender), "amount": amount.to_string() }),
        evm::Decoded::Call { selector } if selector == SEL_AAVE_SUPPLY && t.data.len() == 4 + 128 => {
            let d = &t.data[4..];
            json!({ "kind": "aave_supply", "pool": evm::fmt_address(&t.to), "asset": word_addr(&d[0..32])?, "amount": word_dec(&d[32..64])?, "on_behalf_of": word_addr(&d[64..96])?, "referral": word_dec(&d[96..128])? })
        }
        evm::Decoded::Call { selector } => json!({ "kind": "unknown_call", "contract": evm::fmt_address(&t.to), "selector": format!("0x{}", hex::encode(selector)), "data": format!("0x{}", hex::encode(&t.data)) }),
    };
    serde_json::to_string(&json!({
        "chain_id": t.chain_id,
        "nonce": t.nonce,
        "to": evm::fmt_address(&t.to),
        "value": t.value.to_string(),
        "gas_limit": t.gas_limit,
        "max_fee_per_gas": t.max_fee_per_gas.to_string(),
        "call": call,
        "hash": hex::encode(tx_hash(&t)),
    }))
    .map_err(err)
}

/// The message by which the passkey has the wallet itself ask the MPC
/// signer to sign this EVM transaction, on the wallet's one EVM path
/// (`evm`). The hash is computed here from the transaction, never taken
/// from anyone else.
#[wasm_bindgen]
pub fn owner_evm_sign_message(chain_id: &str, wallet: &str, nonce: u32, created_at_secs: i64, signer: &str, tx_json: &str) -> Result<String, String> {
    let hash = tx_hash(&parse_tx(tx_json)?);
    let args = serde_json::json!({ "request": { "path": "evm", "payload_v2": { "Ecdsa": hex::encode(hash) }, "domain_id": 0 } });
    call_message(chain_id, wallet, nonce, created_at_secs, signer, "sign", &args.to_string(), "1", 30)
}

// ---- Broadcasting what the owner signed ----

fn rlp_len_prefix(out: &mut Vec<u8>, len: usize, base: u8) {
    if len <= 55 {
        out.push(base + len as u8);
    } else {
        let be = (len as u64).to_be_bytes();
        let first = be.iter().position(|b| *b != 0).unwrap_or(be.len());
        out.push(base + 55 + (be.len() - first) as u8);
        out.extend_from_slice(&be[first..]);
    }
}

/// An RLP integer from big-endian bytes (leading zeros dropped).
fn rlp_int(out: &mut Vec<u8>, be: &[u8]) {
    let first = be.iter().position(|b| *b != 0).unwrap_or(be.len());
    let b = &be[first..];
    if b.len() == 1 && b[0] < 0x80 {
        out.push(b[0]);
    } else {
        rlp_len_prefix(out, b.len(), 0x80);
        out.extend_from_slice(b);
    }
}

/// The signed EIP-1559 transaction for an MPC signature
/// (`{ big_r: { affine_point }, s: { scalar }, recovery_id }`, as the
/// signer returns it). S is normalised to low S, as Ethereum requires. The
/// sender is recovered here, so the page broadcasts only a transaction
/// signed by the address it expects.
#[wasm_bindgen]
pub fn evm_signed_tx(tx_json: &str, mpc_signature_json: &str) -> Result<String, String> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
    use sha3::{Digest, Keccak256};
    let t = parse_tx(tx_json)?;
    let hash = tx_hash(&t);
    let sig: serde_json::Value = serde_json::from_str(mpc_signature_json).map_err(err)?;
    let big_r = hex::decode(sig["big_r"]["affine_point"].as_str().ok_or("big_r")?).map_err(err)?;
    let s = hex::decode(sig["s"]["scalar"].as_str().ok_or("s")?).map_err(err)?;
    let mut recid = sig["recovery_id"].as_u64().ok_or("recovery_id")? as u8;
    if big_r.len() != 33 || s.len() != 32 || recid > 1 {
        return Err("bad signature shape".into());
    }
    let mut rs = big_r[1..].to_vec();
    rs.extend(&s);
    let mut signature = Signature::from_slice(&rs).map_err(err)?;
    if let Some(low) = signature.normalize_s() {
        signature = low;
        recid ^= 1;
    }
    let key = VerifyingKey::recover_from_prehash(&hash, &signature, RecoveryId::from_byte(recid).ok_or("recovery id")?)
        .map_err(|_| "signature does not recover".to_string())?;
    let point = key.to_encoded_point(false);
    let from = format!("0x{}", hex::encode(&Keccak256::digest(&point.as_bytes()[1..])[12..]));

    // Unsigned list items, then y-parity, r, s.
    let payload = t.rlp_payload();
    let skip = if payload[0] <= 0xf7 { 1 } else { 1 + (payload[0] - 0xf7) as usize };
    let mut items = payload[skip..].to_vec();
    rlp_int(&mut items, &[recid]);
    let (r_bytes, s_bytes) = signature.split_bytes();
    rlp_int(&mut items, &r_bytes);
    rlp_int(&mut items, &s_bytes);
    let mut raw = vec![0x02];
    rlp_len_prefix(&mut raw, items.len(), 0xc0);
    raw.extend(items);
    serde_json::to_string(&serde_json::json!({
        "raw": format!("0x{}", hex::encode(&raw)),
        "from": from,
        "tx_hash": format!("0x{}", hex::encode(Keccak256::digest(&raw))),
    }))
    .map_err(err)
}

/// Builds the call data and transaction fields for an owner's send: the
/// native coin when `token` is empty, else ERC-20 `transfer`.
#[wasm_bindgen]
#[allow(clippy::too_many_arguments)]
pub fn evm_send_tx(chain_id: u64, nonce: u64, max_priority_fee_per_gas: &str, max_fee_per_gas: &str, gas_limit: u64, token: &str, to: &str, amount: &str) -> Result<String, String> {
    let recipient = evm::parse_address(to)?;
    let amount_u: u128 = amount.parse().map_err(err)?;
    let (target, value, data) = if token.is_empty() {
        (recipient, amount_u, String::from("0x"))
    } else {
        let mut d = evm::SEL_TRANSFER.to_vec();
        d.extend([0u8; 12]);
        d.extend(recipient);
        d.extend([0u8; 16]);
        d.extend(amount_u.to_be_bytes());
        (evm::parse_address(token)?, 0, format!("0x{}", hex::encode(d)))
    };
    let tx = serde_json::json!({
        "chain_id": chain_id, "nonce": nonce,
        "max_priority_fee_per_gas": max_priority_fee_per_gas, "max_fee_per_gas": max_fee_per_gas,
        "gas_limit": gas_limit, "to": evm::fmt_address(&target), "value": value.to_string(), "data": data,
    });
    // Round-trip through the parser so a bad field fails here.
    parse_tx(&tx.to_string())?;
    Ok(tx.to_string())
}

#[cfg(test)]
mod tests;
