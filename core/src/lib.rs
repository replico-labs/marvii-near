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

/// The owner sends NEAR: one transfer to `receiver` (any account, a name
/// included). Confirm shows it from the decoded message.
#[wasm_bindgen]
pub fn transfer_message(chain_id: &str, wallet: &str, nonce: u32, created_at_secs: i64, receiver: &str, yocto: &str) -> Result<String, String> {
    use defuse_wallet::{NearPromise, NearToken, Request, Timestamp};
    let amount: u128 = yocto.parse().map_err(err)?;
    if amount == 0 {
        return Err("nothing to send".into());
    }
    if receiver == wallet {
        return Err("sending to yourself".into());
    }
    let promise = NearPromise::new(receiver.parse::<defuse_wallet::AccountId>().map_err(|_| "not a NEAR account".to_string())?).transfer(NearToken::from_yoctonear(amount));
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

/// The passkey moves the wallet to another extension: adds `add`, removes
/// each of `remove_json` (a JSON list). The address doesn't change. Sessions
/// live in each extension, so the old one's sessions stop working.
#[wasm_bindgen]
pub fn extension_move_message(chain_id: &str, wallet: &str, nonce: u32, created_at_secs: i64, add: &str, remove_json: &str) -> Result<String, String> {
    use defuse_wallet::{Request, Timestamp, WalletOp};
    let remove: Vec<String> = serde_json::from_str(remove_json).map_err(err)?;
    let mut internal = vec![WalletOp::AddExtension { account_id: add.parse().map_err(err)? }];
    for r in remove {
        if r == add {
            return Err("can't remove the extension being added".into());
        }
        internal.push(WalletOp::RemoveExtension { account_id: r.parse().map_err(err)? });
    }
    let msg = RequestMessage {
        pay_for_gas: false,
        chain_id: chain_id.to_string(),
        signer_id: wallet.parse().map_err(err)?,
        nonce,
        created_at: Timestamp::from_secs(created_at_secs).ok_or("bad time")?,
        timeout: core::time::Duration::from_secs(600),
        request: Request { internal, external: vec![] },
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
mod legacy;
mod cardano;
mod hyperliquid;
pub use hyperliquid::*;
pub use legacy::*;
pub use cardano::*;

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
        evm::Decoded::Call { selector } if selector == evm::SEL_AAVE_WITHDRAW && t.data.len() == 4 + 96 => {
            let d = &t.data[4..];
            json!({ "kind": "aave_withdraw", "pool": evm::fmt_address(&t.to), "asset": word_addr(&d[0..32])?, "amount": word_dec(&d[32..64])?, "to": word_addr(&d[64..96])? })
        }
        evm::Decoded::Call { selector } if selector == evm::SEL_AAVE_REPAY && t.data.len() == 4 + 128 => {
            let d = &t.data[4..];
            json!({ "kind": "aave_repay", "pool": evm::fmt_address(&t.to), "asset": word_addr(&d[0..32])?, "amount": word_dec(&d[32..64])?, "on_behalf_of": word_addr(&d[96..128])? })
        }
        evm::Decoded::Call { selector } if selector == evm::SEL_UNI_EXACT_INPUT_SINGLE && t.data.len() == 4 + 224 => {
            let d = &t.data[4..];
            json!({ "kind": "uniswap_swap", "router": evm::fmt_address(&t.to), "token_in": word_addr(&d[0..32])?, "token_out": word_addr(&d[32..64])?, "fee": word_dec(&d[64..96])?, "recipient": word_addr(&d[96..128])?, "amount_in": word_dec(&d[128..160])?, "min_out": word_dec(&d[160..192])? })
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
    let low = signature.normalize_s();
    if low != signature {
        signature = low;
        recid ^= 1;
    }
    let key = VerifyingKey::recover_from_prehash(&hash, &signature, RecoveryId::from_byte(recid).ok_or("recovery id")?)
        .map_err(|_| "signature does not recover".to_string())?;
    let point = key.to_sec1_point(false);
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

// ---- Addresses on other chains ----

/// The derivation path and key domain each chain uses: one path per chain,
/// `domain_id` 0 (secp256k1) or 1 (ed25519).
#[wasm_bindgen]
pub fn chain_path(chain: &str) -> Result<String, String> {
    let (path, domain) = match chain {
        "bitcoin" | "litecoin" | "dogecoin" | "bitcoin-cash" | "zcash" | "tron" | "xrp" => (chain, 0),
        "solana" | "cardano" => (chain, 1),
        _ => return Err(format!("unknown chain {chain}")),
    };
    Ok(serde_json::json!({ "path": path, "domain_id": domain }).to_string())
}

fn hash160(b: &[u8]) -> [u8; 20] {
    use ripemd::Ripemd160;
    use sha2::{Digest, Sha256};
    Ripemd160::digest(Sha256::digest(b)).into()
}

fn b58check(version: &[u8], payload: &[u8]) -> String {
    bs58::encode([version, payload].concat()).with_check().into_string()
}

pub(crate) fn cashaddr_polymod(v: &[u8]) -> u64 {
    let mut c: u64 = 1;
    for d in v {
        let c0 = (c >> 35) as u8;
        c = ((c & 0x07_ffff_ffff) << 5) ^ (*d as u64);
        for (bit, g) in [0x98f2bc8e61u64, 0x79b76d99e2, 0xf33e5fb3c4, 0xae2eabe2a8, 0x1e4f43e470].iter().enumerate() {
            if c0 & (1 << bit) != 0 {
                c ^= g;
            }
        }
    }
    c ^ 1
}

/// CashAddr (Bitcoin Cash), P2PKH: polymod checksum over the prefix.
fn cashaddr(prefix: &str, hash: &[u8; 20]) -> String {
    cashaddr_typed(prefix, 0, hash)
}

/// CashAddr with a version byte: 0 for P2PKH, 8 for P2SH (160-bit hashes).
pub(crate) fn cashaddr_typed(prefix: &str, version: u8, hash: &[u8; 20]) -> String {
    const CHARSET: &[u8] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";
    let mut payload = vec![version];
    payload.extend(hash);
    // 8-bit to 5-bit groups, padded.
    let (mut acc, mut bits, mut five) = (0u32, 0u32, vec![]);
    for b in payload {
        acc = (acc << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            five.push(((acc >> bits) & 31) as u8);
        }
    }
    if bits > 0 {
        five.push(((acc << (5 - bits)) & 31) as u8);
    }
    let mut data: Vec<u8> = prefix.bytes().map(|b| b & 31).collect();
    data.push(0);
    data.extend(&five);
    data.extend([0u8; 8]);
    let m = cashaddr_polymod(&data);
    let check: Vec<u8> = (0..8).map(|i| ((m >> (5 * (7 - i))) & 31) as u8).collect();
    let body: String = five.iter().chain(check.iter()).map(|d| CHARSET[*d as usize] as char).collect();
    format!("{prefix}:{body}")
}

/// The account's address on a non-EVM chain, from the MPC signer's derived
/// key for that chain's path (`secp256k1:<base58 x||y>` or
/// `ed25519:<base58>`). Testnet encodings; each is checked against the
/// chain's own reference library in the tests.
#[wasm_bindgen]
pub fn chain_address(chain: &str, derived_public_key: &str) -> Result<String, String> {
    use sha3::{Digest, Keccak256};
    if let Some(b) = derived_public_key.strip_prefix("secp256k1:") {
        let xy = bs58::decode(b).into_vec().map_err(err)?;
        if xy.len() != 64 {
            return Err("expected 64 bytes".into());
        }
        let mut compressed = vec![if xy[63] & 1 == 0 { 2 } else { 3 }];
        compressed.extend(&xy[..32]);
        let h = hash160(&compressed);
        return Ok(match chain {
            "bitcoin" => bech32::segwit::encode(bech32::Hrp::parse("tb").unwrap(), bech32::segwit::VERSION_0, &h).map_err(err)?,
            "litecoin" => bech32::segwit::encode(bech32::Hrp::parse("tltc").unwrap(), bech32::segwit::VERSION_0, &h).map_err(err)?,
            "dogecoin" => b58check(&[0x71], &h),
            "bitcoin-cash" => cashaddr("bchtest", &h),
            "zcash" => b58check(&[0x1d, 0x25], &h),
            "tron" => b58check(&[0x41], &Keccak256::digest(&xy)[12..]),
            "xrp" => bs58::encode([[0u8].as_slice(), &h].concat()).with_alphabet(bs58::Alphabet::RIPPLE).with_check().into_string(),
            _ => return Err(format!("{chain} doesn't use a secp256k1 key")),
        });
    }
    if let Some(b) = derived_public_key.strip_prefix("ed25519:") {
        let pk = bs58::decode(b).into_vec().map_err(err)?;
        if pk.len() != 32 {
            return Err("expected 32 bytes".into());
        }
        return Ok(match chain {
            "solana" => bs58::encode(&pk).into_string(),
            "cardano" => {
                use blake2::{Blake2bVar, digest::{Update, VariableOutput}};
                let mut h = Blake2bVar::new(28).unwrap();
                h.update(&pk);
                let mut kh = [0u8; 28];
                h.finalize_variable(&mut kh).unwrap();
                // CIP-19 enterprise address, key hash, testnet: header 0x60.
                let mut bytes = vec![0x60];
                bytes.extend(kh);
                bech32::encode::<bech32::Bech32>(bech32::Hrp::parse("addr_test").unwrap(), &bytes).map_err(err)?
            }
            _ => return Err(format!("{chain} doesn't use an ed25519 key")),
        });
    }
    Err("unknown key type".into())
}

// ---- Solana: the owner sends SOL ----

fn compact_u16(out: &mut Vec<u8>, mut v: usize) {
    loop {
        let mut b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        b |= 0x80;
        out.push(b);
    }
}

fn b58_32(s: &str) -> Result<[u8; 32], String> {
    bs58::decode(s).into_vec().map_err(err)?.try_into().map_err(|_| "expected 32 bytes".to_string())
}

/// System Program (all zero bytes).
const SOL_SYSTEM_PROGRAM: [u8; 32] = [0; 32];

/// A legacy Solana message for one System Program transfer, as hex: the
/// exact bytes the account's ed25519 key signs.
#[wasm_bindgen]
pub fn solana_transfer_message(from: &str, to: &str, lamports: &str, recent_blockhash: &str) -> Result<String, String> {
    let (from, to, bh) = (b58_32(from)?, b58_32(to)?, b58_32(recent_blockhash)?);
    if from == to {
        return Err("sending to yourself".into());
    }
    let lamports: u64 = lamports.parse().map_err(err)?;
    // Header: 1 signature; 0 read-only signed; 1 read-only unsigned (the program).
    let mut m = vec![1u8, 0, 1];
    compact_u16(&mut m, 3);
    m.extend(from);
    m.extend(to);
    m.extend(SOL_SYSTEM_PROGRAM);
    m.extend(bh);
    compact_u16(&mut m, 1); // one instruction
    m.push(2); // program: System
    compact_u16(&mut m, 2);
    m.extend([0u8, 1]); // from, to
    let mut data = 2u32.to_le_bytes().to_vec(); // Transfer
    data.extend(lamports.to_le_bytes());
    compact_u16(&mut m, data.len());
    m.extend(data);
    Ok(hex::encode(m))
}

/// Decodes a message for Confirm. Only the exact single-transfer layout
/// above is recognised; anything else is refused, never guessed.
#[wasm_bindgen]
pub fn describe_solana_message(message_hex: &str) -> Result<String, String> {
    let m = hex::decode(message_hex).map_err(err)?;
    if m.len() != 3 + 1 + 96 + 32 + 1 + 1 + 1 + 2 + 1 + 12 || m[..4] != [1, 0, 1, 3] {
        return Err("not a plain SOL transfer".into());
    }
    let from = &m[4..36];
    let to = &m[36..68];
    let i = 4 + 96 + 32;
    if m[68..100] != SOL_SYSTEM_PROGRAM || m[i..i + 6] != [1, 2, 2, 0, 1, 12] || m[i + 6..i + 10] != [2, 0, 0, 0] {
        return Err("not a plain SOL transfer".into());
    }
    let lamports = u64::from_le_bytes(m[i + 10..i + 18].try_into().unwrap());
    serde_json::to_string(&serde_json::json!({
        "kind": "sol_transfer",
        "from": bs58::encode(from).into_string(),
        "to": bs58::encode(to).into_string(),
        "lamports": lamports.to_string(),
    }))
    .map_err(err)
}

/// The message by which the passkey has the wallet ask the MPC signer for
/// an ed25519 signature over `message_hex`, on the given chain's path.
#[wasm_bindgen]
pub fn owner_eddsa_sign_message(chain_id: &str, wallet: &str, nonce: u32, created_at_secs: i64, signer: &str, chain: &str, message_hex: &str) -> Result<String, String> {
    let p: serde_json::Value = serde_json::from_str(&chain_path(chain)?).map_err(err)?;
    if p["domain_id"] != 1 {
        return Err(format!("{chain} doesn't use ed25519"));
    }
    let bytes = hex::decode(message_hex).map_err(err)?;
    if bytes.is_empty() || bytes.len() > 1232 {
        return Err("message must be 1 to 1232 bytes".into());
    }
    let args = serde_json::json!({ "request": { "path": chain, "payload_v2": { "Eddsa": hex::encode(&bytes) }, "domain_id": 1 } });
    call_message(chain_id, wallet, nonce, created_at_secs, signer, "sign", &args.to_string(), "1", 30)
}

/// The signed Solana transaction (base64, for `sendTransaction`), after
/// checking the MPC signature (`{ scheme: "Ed25519", signature: [64] }`)
/// verifies for `from` over exactly this message.
#[wasm_bindgen]
pub fn solana_signed_tx(message_hex: &str, mpc_signature_json: &str, from: &str) -> Result<String, String> {
    use base64::Engine;
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let m = hex::decode(message_hex).map_err(err)?;
    let sig: serde_json::Value = serde_json::from_str(mpc_signature_json).map_err(err)?;
    let bytes: Vec<u8> = sig["signature"].as_array().ok_or("signature")?.iter().map(|v| v.as_u64().unwrap_or(256) as u8).collect();
    let bytes: [u8; 64] = bytes.try_into().map_err(|_| "signature must be 64 bytes".to_string())?;
    let key = VerifyingKey::from_bytes(&b58_32(from)?).map_err(err)?;
    key.verify(&m, &Signature::from_bytes(&bytes)).map_err(|_| "signature is not from this account".to_string())?;
    let mut tx = vec![];
    compact_u16(&mut tx, 1);
    tx.extend(bytes);
    tx.extend(m);
    Ok(base64::engine::general_purpose::STANDARD.encode(tx))
}

// ---- Bitcoin: the owner sends BTC (native SegWit, testnet4) ----

fn sha256d(b: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(Sha256::digest(b)).into()
}

fn varint(out: &mut Vec<u8>, n: usize) {
    match n {
        0..=0xfc => out.push(n as u8),
        0xfd..=0xffff => {
            out.push(0xfd);
            out.extend((n as u16).to_le_bytes());
        }
        _ => {
            out.push(0xfe);
            out.extend((n as u32).to_le_bytes());
        }
    }
}

/// Which Bitcoin-style chain an account address is on, by its SegWit prefix:
/// Bitcoin testnet4 (`tb1…`) or Litecoin testnet (`tltc1…`). The same plan,
/// signing hashes (BIP-143) and transaction format serve both.
fn btc_chain(from: &str) -> Result<(&'static str, &'static str, &'static str), String> {
    let a = from.to_lowercase();
    if a.starts_with("tltc1") {
        Ok(("litecoin", "tltc", "LTC"))
    } else if a.starts_with("tb1") {
        Ok(("bitcoin", "tb", "BTC"))
    } else {
        Err("sending address must be native SegWit (P2WPKH)".into())
    }
}

/// The output script for a testnet address on the chain with SegWit prefix
/// `hrp`: SegWit (any version), legacy P2PKH (`m…`/`n…`) or P2SH (`2…`, and
/// Litecoin's `Q…`). Legacy testnet addresses look the same on both chains.
fn btc_script(address: &str, hrp_want: &str) -> Result<Vec<u8>, String> {
    let bad = || format!("not a {} testnet address", if hrp_want == "tltc" { "Litecoin" } else { "Bitcoin" });
    if address.to_lowercase().starts_with(&format!("{hrp_want}1")) {
        let (hrp, version, program) = bech32::segwit::decode(address).map_err(|_| bad())?;
        if hrp.as_str() != hrp_want {
            return Err(bad());
        }
        let v = version.to_u8();
        let mut s = vec![if v == 0 { 0 } else { 0x50 + v }, program.len() as u8];
        s.extend(program);
        return Ok(s);
    }
    let raw = bs58::decode(address).with_check(None).into_vec().map_err(|_| bad())?;
    if raw.len() != 21 {
        return Err(bad());
    }
    Ok(match raw[0] {
        0x6f => [&[0x76, 0xa9, 0x14][..], &raw[1..], &[0x88, 0xac]].concat(),
        0xc4 => [&[0xa9, 0x14][..], &raw[1..], &[0x87]].concat(),
        0x3a if hrp_want == "tltc" => [&[0xa9, 0x14][..], &raw[1..], &[0x87]].concat(),
        _ => return Err(bad()),
    })
}

pub(crate) fn btc_script_pub(address: &str, hrp: &str) -> Result<Vec<u8>, String> {
    btc_script(address, hrp)
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct BtcInput {
    txid: String,
    vout: u32,
    value: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct BtcOutput {
    address: String,
    value: u64,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct BtcPlan {
    from: String,
    inputs: Vec<BtcInput>,
    outputs: Vec<BtcOutput>,
    fee: u64,
    /// One BIP-143 signing hash per input, hex.
    sighashes: Vec<String>,
}

const BTC_SEQUENCE: u32 = 0xffff_fffd; // replaceable
const BTC_DUST: u64 = 546;
/// Litecoin: a deliberately high limit (0.001 LTC), since Litecoin Core's own
/// dust rule wasn't checked; change below it goes to the fee instead.
const LTC_DUST: u64 = 100_000;

fn txid_bytes(txid: &str) -> Result<Vec<u8>, String> {
    let mut b = hex::decode(txid).map_err(err)?;
    if b.len() != 32 {
        return Err("bad txid".into());
    }
    b.reverse();
    Ok(b)
}

fn btc_sighashes(from: &str, inputs: &[BtcInput], outputs: &[BtcOutput]) -> Result<Vec<String>, String> {
    let (_, hrp, _) = btc_chain(from)?;
    let program = btc_script(from, hrp)?;
    if program.len() != 22 || program[0] != 0 {
        return Err("sending address must be native SegWit (P2WPKH)".into());
    }
    let mut prevouts = vec![];
    let mut seqs = vec![];
    for i in inputs {
        prevouts.extend(txid_bytes(&i.txid)?);
        prevouts.extend(i.vout.to_le_bytes());
        seqs.extend(BTC_SEQUENCE.to_le_bytes());
    }
    let mut outs = vec![];
    for o in outputs {
        outs.extend(o.value.to_le_bytes());
        let s = btc_script(&o.address, hrp)?;
        varint(&mut outs, s.len());
        outs.extend(s);
    }
    let (hp, hs, ho) = (sha256d(&prevouts), sha256d(&seqs), sha256d(&outs));
    let script_code = [&[0x19, 0x76, 0xa9, 0x14][..], &program[2..], &[0x88, 0xac]].concat();
    inputs
        .iter()
        .map(|i| {
            let mut pre = 2u32.to_le_bytes().to_vec();
            pre.extend(hp);
            pre.extend(hs);
            pre.extend(txid_bytes(&i.txid)?);
            pre.extend(i.vout.to_le_bytes());
            pre.extend(&script_code);
            pre.extend(i.value.to_le_bytes());
            pre.extend(BTC_SEQUENCE.to_le_bytes());
            pre.extend(ho);
            pre.extend(0u32.to_le_bytes()); // locktime
            pre.extend(1u32.to_le_bytes()); // SIGHASH_ALL
            Ok(hex::encode(sha256d(&pre)))
        })
        .collect()
}

/// Plans a send: picks coins (largest first) from `utxos_json`
/// (`[{txid, vout, value}]`, as mempool.space lists them), pays `to`, and
/// returns change to `from` unless it would be dust. At most 6 inputs.
/// `from` picks the chain (Bitcoin testnet4 or Litecoin testnet).
#[wasm_bindgen]
pub fn bitcoin_plan(from: &str, to: &str, amount_sat: &str, utxos_json: &str, fee_rate_sat_vb: u32) -> Result<String, String> {
    let amount: u64 = amount_sat.parse().map_err(err)?;
    let (chain, hrp, sym) = btc_chain(from)?;
    let dust = if chain == "litecoin" { LTC_DUST } else { BTC_DUST };
    if amount < dust {
        return Err("amount below the dust limit".into());
    }
    let to_script = btc_script(to, hrp)?;
    let mut utxos: Vec<BtcInput> = serde_json::from_str(utxos_json).map_err(err)?;
    utxos.sort_by(|a, b| b.value.cmp(&a.value));
    let vbytes = |n_in: usize, change: bool| -> u64 {
        // P2WPKH inputs ~68 vB; outputs 8 + 1 + script; ~11 vB overhead.
        11 + 68 * n_in as u64 + (9 + to_script.len() as u64) + if change { 31 } else { 0 }
    };
    let mut inputs = vec![];
    let mut total = 0u64;
    for u in utxos {
        if inputs.len() == 6 {
            break;
        }
        total += u.value;
        inputs.push(u);
        let fee = vbytes(inputs.len(), true) * fee_rate_sat_vb as u64;
        if total >= amount + fee {
            break;
        }
    }
    let fee_with_change = vbytes(inputs.len(), true) * fee_rate_sat_vb as u64;
    let fee_no_change = vbytes(inputs.len(), false) * fee_rate_sat_vb as u64;
    let mut outputs = vec![BtcOutput { address: to.to_string(), value: amount }];
    let fee = if total >= amount + fee_with_change + dust {
        outputs.push(BtcOutput { address: from.to_string(), value: total - amount - fee_with_change });
        fee_with_change
    } else if total >= amount + fee_no_change {
        total - amount // the small remainder goes to the fee
    } else {
        return Err(format!("not enough {sym} for this amount and fee"));
    };
    let sighashes = btc_sighashes(from, &inputs, &outputs)?;
    serde_json::to_string(&BtcPlan { from: from.to_string(), inputs, outputs, fee, sighashes }).map_err(err)
}

/// For Confirm: what a plan pays, checked again from its parts (the
/// signing hashes are recomputed here, not trusted).
#[wasm_bindgen]
pub fn describe_bitcoin_plan(plan_json: &str) -> Result<String, String> {
    let p: BtcPlan = serde_json::from_str(plan_json).map_err(err)?;
    if btc_sighashes(&p.from, &p.inputs, &p.outputs)? != p.sighashes {
        return Err("signing hashes don't match the transaction".into());
    }
    let total_in: u64 = p.inputs.iter().map(|i| i.value).sum();
    let total_out: u64 = p.outputs.iter().map(|o| o.value).sum();
    if total_in < total_out || total_in - total_out != p.fee {
        return Err("fee doesn't add up".into());
    }
    let pays: Vec<_> = p.outputs.iter().filter(|o| o.address != p.from).map(|o| serde_json::json!({ "to": o.address, "sat": o.value.to_string() })).collect();
    let (chain, _, _) = btc_chain(&p.from)?;
    serde_json::to_string(&serde_json::json!({ "chain": chain, "pays": pays, "fee_sat": p.fee.to_string(), "inputs": p.inputs.len(), "sighashes": p.sighashes })).map_err(err)
}

/// The passkey's request for one ECDSA signature per hash, on a chain's
/// path: one call to the signer per hash.
#[wasm_bindgen]
pub fn owner_ecdsa_multi_sign_message(chain_id: &str, wallet: &str, nonce: u32, created_at_secs: i64, signer: &str, chain: &str, hashes_json: &str) -> Result<String, String> {
    use defuse_wallet::{Gas, NearPromise, NearToken, Request, Timestamp, actions::FunctionCall};
    let p: serde_json::Value = serde_json::from_str(&chain_path(chain)?).map_err(err)?;
    if p["domain_id"] != 0 {
        return Err(format!("{chain} doesn't use secp256k1"));
    }
    let hashes: Vec<String> = serde_json::from_str(hashes_json).map_err(err)?;
    if hashes.is_empty() || hashes.len() > 6 {
        return Err("1 to 6 signatures per request".into());
    }
    let signer_id: defuse_wallet::AccountId = signer.parse().map_err(err)?;
    let mut external = vec![];
    for h in &hashes {
        hex32(h)?;
        let args = serde_json::json!({ "request": { "path": chain, "payload_v2": { "Ecdsa": h }, "domain_id": 0 } });
        external.push(NearPromise::new(signer_id.clone()).function_call(
            FunctionCall::name("sign").args(serde_json::to_vec(&args).map_err(err)?).attach_deposit(NearToken::from_yoctonear(1)).gas(Gas::from_tgas(30)),
        ));
    }
    let msg = RequestMessage {
        pay_for_gas: false,
        chain_id: chain_id.to_string(),
        signer_id: wallet.parse().map_err(err)?,
        nonce,
        created_at: Timestamp::from_secs(created_at_secs).ok_or("bad time")?,
        timeout: core::time::Duration::from_secs(600),
        request: Request { internal: vec![], external },
    };
    serde_json::to_string(&msg).map_err(err)
}

/// The signed transaction (hex) and its txid. Each MPC signature is matched
/// to the input whose hash it signs, and must recover to the key behind
/// `from`; S is made low and DER-encoded.
#[wasm_bindgen]
pub fn bitcoin_signed_tx(plan_json: &str, mpc_signatures_json: &str) -> Result<String, String> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
    let p: BtcPlan = serde_json::from_str(plan_json).map_err(err)?;
    if btc_sighashes(&p.from, &p.inputs, &p.outputs)? != p.sighashes {
        return Err("signing hashes don't match the transaction".into());
    }
    let (_, hrp, _) = btc_chain(&p.from)?;
    let program = btc_script(&p.from, hrp)?;
    let sigs: Vec<serde_json::Value> = serde_json::from_str(mpc_signatures_json).map_err(err)?;
    let mut witnesses: Vec<Option<(Vec<u8>, Vec<u8>)>> = vec![None; p.inputs.len()];
    for sig in &sigs {
        let big_r = hex::decode(sig["big_r"]["affine_point"].as_str().ok_or("big_r")?).map_err(err)?;
        let s = hex::decode(sig["s"]["scalar"].as_str().ok_or("s")?).map_err(err)?;
        let recid = sig["recovery_id"].as_u64().ok_or("recovery_id")? as u8;
        if big_r.len() != 33 || s.len() != 32 || recid > 1 {
            return Err("bad signature shape".into());
        }
        let signature = Signature::from_slice(&[&big_r[1..], &s[..]].concat()).map_err(err)?;
        for (i, h) in p.sighashes.iter().enumerate() {
            let hash = hex32(h)?;
            let Ok(key) = VerifyingKey::recover_from_prehash(&hash, &signature, RecoveryId::from_byte(recid).unwrap()) else { continue };
            let pubkey = key.to_sec1_point(true).as_bytes().to_vec();
            if hash160(&pubkey)[..] == program[2..] && witnesses[i].is_none() {
                let low = signature.normalize_s();
                let mut der = low.to_der().as_bytes().to_vec();
                der.push(1); // SIGHASH_ALL
                witnesses[i] = Some((der, pubkey));
                break;
            }
        }
    }
    let witnesses: Vec<_> = witnesses.into_iter().enumerate().map(|(i, w)| w.ok_or(format!("no valid signature for input {i}"))).collect::<Result<_, _>>()?;
    let mut base = 2u32.to_le_bytes().to_vec();
    let mut body = vec![];
    varint(&mut body, p.inputs.len());
    for i in &p.inputs {
        body.extend(txid_bytes(&i.txid)?);
        body.extend(i.vout.to_le_bytes());
        body.push(0);
        body.extend(BTC_SEQUENCE.to_le_bytes());
    }
    varint(&mut body, p.outputs.len());
    for o in &p.outputs {
        body.extend(o.value.to_le_bytes());
        let s = btc_script(&o.address, hrp)?;
        varint(&mut body, s.len());
        body.extend(s);
    }
    let mut legacy = base.clone();
    legacy.extend(&body);
    legacy.extend(0u32.to_le_bytes());
    base.extend([0, 1]); // segwit marker and flag
    base.extend(&body);
    for (sig, pk) in &witnesses {
        base.push(2);
        varint(&mut base, sig.len());
        base.extend(sig);
        varint(&mut base, pk.len());
        base.extend(pk);
    }
    base.extend(0u32.to_le_bytes());
    let mut txid = sha256d(&legacy);
    txid.reverse();
    serde_json::to_string(&serde_json::json!({ "raw": hex::encode(base), "txid": hex::encode(txid) })).map_err(err)
}

// ---- TRON: the owner sends TRX (built here, never by a node) ----

const TRON_TRANSFER_TYPE_URL: &[u8] = b"type.googleapis.com/protocol.TransferContract";

fn pb_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn pb_bytes(out: &mut Vec<u8>, field: u8, b: &[u8]) {
    out.push(field << 3 | 2);
    pb_varint(out, b.len() as u64);
    out.extend(b);
}

fn pb_uint(out: &mut Vec<u8>, field: u8, v: u64) {
    out.push(field << 3);
    pb_varint(out, v);
}

fn tron_addr_bytes(a: &str) -> Result<Vec<u8>, String> {
    let b = bs58::decode(a).with_check(None).into_vec().map_err(|_| "not a TRON address".to_string())?;
    if b.len() != 21 || b[0] != 0x41 {
        return Err("not a TRON address".into());
    }
    Ok(b)
}

/// The raw transaction for a TRX transfer, built here from the latest
/// block's reference (`ref_block_bytes` 2 bytes, `ref_block_hash` 8 bytes):
/// `{ raw_hex, txid }`, where txid = sha256(raw) is what gets signed.
#[wasm_bindgen]
pub fn tron_transfer_raw(from: &str, to: &str, amount_sun: &str, ref_block_bytes_hex: &str, ref_block_hash_hex: &str, expiration_ms: u64, timestamp_ms: u64) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let (owner, dest) = (tron_addr_bytes(from)?, tron_addr_bytes(to)?);
    if owner == dest {
        return Err("sending to yourself".into());
    }
    let amount: u64 = amount_sun.parse().map_err(err)?;
    let (rb, rh) = (hex::decode(ref_block_bytes_hex).map_err(err)?, hex::decode(ref_block_hash_hex).map_err(err)?);
    if rb.len() != 2 || rh.len() != 8 || amount == 0 {
        return Err("bad block reference or amount".into());
    }
    let mut transfer = vec![];
    pb_bytes(&mut transfer, 1, &owner);
    pb_bytes(&mut transfer, 2, &dest);
    pb_uint(&mut transfer, 3, amount);
    let mut any = vec![];
    pb_bytes(&mut any, 1, TRON_TRANSFER_TYPE_URL);
    pb_bytes(&mut any, 2, &transfer);
    let mut contract = vec![];
    pb_uint(&mut contract, 1, 1); // TransferContract
    pb_bytes(&mut contract, 2, &any);
    let mut raw = vec![];
    pb_bytes(&mut raw, 1, &rb);
    pb_bytes(&mut raw, 4, &rh);
    pb_uint(&mut raw, 8, expiration_ms);
    pb_bytes(&mut raw, 11, &contract);
    pb_uint(&mut raw, 14, timestamp_ms);
    serde_json::to_string(&serde_json::json!({ "raw_hex": hex::encode(&raw), "txid": hex::encode(Sha256::digest(&raw)) })).map_err(err)
}

/// For Confirm: decodes a raw transaction, accepting only the exact
/// single-TRX-transfer layout built above.
#[wasm_bindgen]
pub fn describe_tron_raw(raw_hex: &str) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let raw = hex::decode(raw_hex).map_err(err)?;
    // Re-encode from the decoded fields and require identical bytes.
    let not = || "not a plain TRX transfer".to_string();
    let find = |marker: &[u8]| raw.windows(marker.len()).position(|w| w == marker);
    let o = find(&[0x0a, 0x15, 0x41]).ok_or_else(not)?;
    let owner = &raw[o + 2..o + 23];
    let dest = raw.get(o + 25..o + 46).ok_or_else(not)?;
    let encode = |b: &[u8]| bs58::encode(b).with_check().into_string();
    let rb = raw.get(2..4).ok_or_else(not)?;
    let rh = raw.get(6..14).ok_or_else(not)?;
    // expiration and timestamp: read the varints at their tags.
    let read = |at: usize| -> Result<(u64, usize), String> {
        let (mut v, mut shift, mut j) = (0u64, 0, at);
        loop {
            let b = *raw.get(j).ok_or_else(not)?;
            v |= ((b & 0x7f) as u64) << shift;
            j += 1;
            if b & 0x80 == 0 {
                return Ok((v, j));
            }
            shift += 7;
        }
    };
    if raw.get(14) != Some(&0x40) {
        return Err(not());
    }
    let (expiration, _) = read(15)?;
    let t = raw.len() - raw.iter().rev().position(|b| *b == 0x70).ok_or_else(not)? - 1;
    let (timestamp, _) = read(t + 1)?;
    let a = o + 46;
    if raw.get(a) != Some(&0x18) {
        return Err(not());
    }
    let (amount, _) = read(a + 1)?;
    let rebuilt: serde_json::Value = serde_json::from_str(&tron_transfer_raw(&encode(owner), &encode(dest), &amount.to_string(), &hex::encode(rb), &hex::encode(rh), expiration, timestamp)?).map_err(err)?;
    if rebuilt["raw_hex"] != hex::encode(&raw) {
        return Err(not());
    }
    serde_json::to_string(&serde_json::json!({
        "from": encode(owner), "to": encode(dest), "sun": amount.to_string(),
        "expiration_ms": expiration, "txid": hex::encode(Sha256::digest(&raw)),
    }))
    .map_err(err)
}

/// The signed transaction as protobuf hex (for `broadcasthex`), after
/// checking the MPC signature over the txid recovers to `from`.
#[wasm_bindgen]
pub fn tron_signed_tx(raw_hex: &str, mpc_signature_json: &str, from: &str) -> Result<String, String> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
    use sha3::{Digest as _, Keccak256};
    let d: serde_json::Value = serde_json::from_str(&describe_tron_raw(raw_hex)?).map_err(err)?;
    if d["from"] != from {
        return Err("transaction is not from this account".into());
    }
    let txid = hex32(d["txid"].as_str().unwrap())?;
    let sig: serde_json::Value = serde_json::from_str(mpc_signature_json).map_err(err)?;
    let big_r = hex::decode(sig["big_r"]["affine_point"].as_str().ok_or("big_r")?).map_err(err)?;
    let s = hex::decode(sig["s"]["scalar"].as_str().ok_or("s")?).map_err(err)?;
    let mut recid = sig["recovery_id"].as_u64().ok_or("recovery_id")? as u8;
    if big_r.len() != 33 || s.len() != 32 || recid > 1 {
        return Err("bad signature shape".into());
    }
    let mut signature = Signature::from_slice(&[&big_r[1..], &s[..]].concat()).map_err(err)?;
    let low = signature.normalize_s();
    if low != signature {
        signature = low;
        recid ^= 1;
    }
    let key = VerifyingKey::recover_from_prehash(&txid, &signature, RecoveryId::from_byte(recid).unwrap()).map_err(|_| "signature does not recover".to_string())?;
    let point = key.to_sec1_point(false);
    let addr = bs58::encode([&[0x41u8][..], &Keccak256::digest(&point.as_bytes()[1..])[12..]].concat()).with_check().into_string();
    if addr != from {
        return Err("signature is not from this account".into());
    }
    let mut sig65 = signature.to_bytes().to_vec();
    sig65.push(27 + recid);
    let mut tx = vec![];
    pb_bytes(&mut tx, 1, &hex::decode(raw_hex).map_err(err)?);
    pb_bytes(&mut tx, 2, &sig65);
    serde_json::to_string(&serde_json::json!({ "tx_hex": hex::encode(tx), "txid": d["txid"], "signature": hex::encode(sig65) })).map_err(err)
}

fn xrp_account_id(a: &str) -> Result<[u8; 20], String> {
    let b = bs58::decode(a).with_alphabet(bs58::Alphabet::RIPPLE).with_check(None).into_vec().map_err(|_| "not an XRP address (use the r… form)".to_string())?;
    if b.len() != 21 || b[0] != 0 {
        return Err("not an XRP address (use the r… form)".into());
    }
    Ok(b[1..].try_into().unwrap())
}

fn xrp_address(id: &[u8]) -> String {
    bs58::encode([[0u8].as_slice(), id].concat()).with_alphabet(bs58::Alphabet::RIPPLE).with_check().into_string()
}

fn sha512_half(parts: &[&[u8]]) -> [u8; 32] {
    use sha2::{Digest, Sha512};
    let mut h = Sha512::new();
    for p in parts {
        h.update(p);
    }
    h.finalize()[..32].try_into().unwrap()
}

/// Highest fee accepted for an XRP send: 0.1 XRP. The network's normal fee
/// is 12 drops; this stops a dishonest node's fee quote from draining funds.
const XRP_MAX_FEE_DROPS: u64 = 100_000;

/// The canonical binary of an XRP Payment, in the field order the ledger
/// requires (by type code, then field code). `signature` is the DER
/// TxnSignature, or None for the bytes that get signed.
fn xrp_payment_bytes(from: &[u8; 20], to: &[u8; 20], drops: u64, fee: u64, sequence: u32, last_ledger: u32, tag: Option<u32>, pubkey: &[u8], signature: Option<&[u8]>) -> Vec<u8> {
    let mut b = vec![0x12, 0x00, 0x00]; // TransactionType: Payment
    b.push(0x24); // Sequence
    b.extend(sequence.to_be_bytes());
    if let Some(t) = tag {
        b.push(0x2e); // DestinationTag
        b.extend(t.to_be_bytes());
    }
    b.extend([0x20, 0x1b]); // LastLedgerSequence
    b.extend(last_ledger.to_be_bytes());
    b.push(0x61); // Amount (XRP: positive bit set)
    b.extend((0x4000_0000_0000_0000u64 | drops).to_be_bytes());
    b.push(0x68); // Fee
    b.extend((0x4000_0000_0000_0000u64 | fee).to_be_bytes());
    b.push(0x73); // SigningPubKey
    b.push(pubkey.len() as u8);
    b.extend(pubkey);
    if let Some(sig) = signature {
        b.push(0x74); // TxnSignature
        b.push(sig.len() as u8);
        b.extend(sig);
    }
    b.extend([0x81, 0x14]); // Account
    b.extend(from);
    b.extend([0x83, 0x14]); // Destination
    b.extend(to);
    b
}

/// An XRP Payment built here: `{ tx_hex, signing_hash }`. The signing key
/// is the account's derived key (`secp256k1:<base58 x||y>`), and it must
/// hash to `from`. `destination_tag` is optional (pass a negative number for none).
#[wasm_bindgen]
pub fn xrp_payment(from: &str, derived_public_key: &str, to: &str, drops: &str, fee_drops: &str, sequence: u32, last_ledger_sequence: u32, destination_tag: f64) -> Result<String, String> {
    let (a, d) = (xrp_account_id(from)?, xrp_account_id(to)?);
    if a == d {
        return Err("sending to yourself".into());
    }
    let xy = bs58::decode(derived_public_key.strip_prefix("secp256k1:").ok_or("expected a secp256k1 key")?).into_vec().map_err(err)?;
    if xy.len() != 64 {
        return Err("expected 64 bytes".into());
    }
    let mut pubkey = vec![if xy[63] & 1 == 0 { 2 } else { 3 }];
    pubkey.extend(&xy[..32]);
    if hash160(&pubkey) != a {
        return Err("the key is not this account's".into());
    }
    let amount: u64 = drops.parse().map_err(err)?;
    let fee: u64 = fee_drops.parse().map_err(err)?;
    if amount == 0 || amount > 100_000_000_000_000_000 {
        return Err("bad amount".into());
    }
    if fee == 0 || fee > XRP_MAX_FEE_DROPS {
        return Err(format!("fee of {fee} drops is outside 1..{XRP_MAX_FEE_DROPS}"));
    }
    let tag = if destination_tag < 0.0 {
        None
    } else if destination_tag.fract() == 0.0 && destination_tag <= u32::MAX as f64 {
        Some(destination_tag as u32)
    } else {
        return Err("bad destination tag".into());
    };
    let tx = xrp_payment_bytes(&a, &d, amount, fee, sequence, last_ledger_sequence, tag, &pubkey, None);
    serde_json::to_string(&serde_json::json!({ "tx_hex": hex::encode(&tx), "signing_hash": hex::encode(sha512_half(&[b"STX\0", &tx])) })).map_err(err)
}

/// For Confirm: decodes an unsigned Payment, accepting only the exact layout
/// built above (re-encoded and compared byte for byte).
#[wasm_bindgen]
pub fn describe_xrp_tx(tx_hex: &str) -> Result<String, String> {
    let tx = hex::decode(tx_hex).map_err(err)?;
    let not = || "not a plain XRP payment".to_string();
    let take = |at: &mut usize, n: usize| -> Result<Vec<u8>, String> {
        let v = tx.get(*at..*at + n).ok_or_else(not)?.to_vec();
        *at += n;
        Ok(v)
    };
    let u32_at = |v: Vec<u8>| u32::from_be_bytes(v.try_into().unwrap());
    let mut i = 0;
    if take(&mut i, 3)? != [0x12, 0, 0] || take(&mut i, 1)? != [0x24] {
        return Err(not());
    }
    let sequence = u32_at(take(&mut i, 4)?);
    let tag = if tx.get(i) == Some(&0x2e) {
        i += 1;
        Some(u32_at(take(&mut i, 4)?))
    } else {
        None
    };
    if take(&mut i, 2)? != [0x20, 0x1b] {
        return Err(not());
    }
    let last_ledger = u32_at(take(&mut i, 4)?);
    let amount = |i: &mut usize, tag: u8| -> Result<u64, String> {
        if take(i, 1)? != [tag] {
            return Err(not());
        }
        let v = u64::from_be_bytes(take(i, 8)?.try_into().unwrap());
        if v >> 62 != 1 {
            return Err(not()); // not a positive XRP amount
        }
        Ok(v & 0x3fff_ffff_ffff_ffff)
    };
    let drops = amount(&mut i, 0x61)?;
    let fee = amount(&mut i, 0x68)?;
    if take(&mut i, 2)? != [0x73, 33] {
        return Err(not());
    }
    let pubkey = take(&mut i, 33)?;
    if take(&mut i, 2)? != [0x81, 0x14] {
        return Err(not());
    }
    let from: [u8; 20] = take(&mut i, 20)?.try_into().unwrap();
    if take(&mut i, 2)? != [0x83, 0x14] {
        return Err(not());
    }
    let to: [u8; 20] = take(&mut i, 20)?.try_into().unwrap();
    if hash160(&pubkey) != from || fee == 0 || fee > XRP_MAX_FEE_DROPS || drops == 0 {
        return Err(not());
    }
    if xrp_payment_bytes(&from, &to, drops, fee, sequence, last_ledger, tag, &pubkey, None) != tx {
        return Err(not());
    }
    serde_json::to_string(&serde_json::json!({
        "from": xrp_address(&from), "to": xrp_address(&to), "drops": drops.to_string(), "fee_drops": fee.to_string(),
        "destination_tag": tag, "sequence": sequence, "last_ledger_sequence": last_ledger,
        "signing_hash": hex::encode(sha512_half(&[b"STX\0", &tx])),
    }))
    .map_err(err)
}

/// The signed transaction blob (uppercase hex, for `submit`) and its hash,
/// after checking the MPC signature verifies under the transaction's own
/// signing key, which belongs to `from`. The signature is made low-S, as the
/// ledger requires.
#[wasm_bindgen]
pub fn xrp_signed_tx(tx_hex: &str, mpc_signature_json: &str, from: &str) -> Result<String, String> {
    use k256::ecdsa::{Signature, VerifyingKey, signature::hazmat::PrehashVerifier};
    let d: serde_json::Value = serde_json::from_str(&describe_xrp_tx(tx_hex)?).map_err(err)?;
    if d["from"] != from {
        return Err("transaction is not from this account".into());
    }
    let tx = hex::decode(tx_hex).map_err(err)?;
    let hash = hex32(d["signing_hash"].as_str().unwrap())?;
    let sig: serde_json::Value = serde_json::from_str(mpc_signature_json).map_err(err)?;
    let big_r = hex::decode(sig["big_r"]["affine_point"].as_str().ok_or("big_r")?).map_err(err)?;
    let s = hex::decode(sig["s"]["scalar"].as_str().ok_or("s")?).map_err(err)?;
    if big_r.len() != 33 || s.len() != 32 {
        return Err("bad signature shape".into());
    }
    let mut signature = Signature::from_slice(&[&big_r[1..], &s[..]].concat()).map_err(err)?;
    signature = signature.normalize_s();
    // The SigningPubKey sits at a fixed place in the layout describe accepted.
    let at = tx.windows(2).position(|w| w == [0x73, 33]).ok_or("no signing key")? + 2;
    let key = VerifyingKey::from_sec1_bytes(&tx[at..at + 33]).map_err(err)?;
    key.verify_prehash(&hash, &signature).map_err(|_| "signature is not from this account".to_string())?;
    let der = signature.to_der();
    let (a, t) = (xrp_account_id(from)?, xrp_account_id(d["to"].as_str().unwrap())?);
    let tag = d["destination_tag"].as_u64().map(|v| v as u32);
    let signed = xrp_payment_bytes(&a, &t, d["drops"].as_str().unwrap().parse().unwrap(), d["fee_drops"].as_str().unwrap().parse().unwrap(), d["sequence"].as_u64().unwrap() as u32, d["last_ledger_sequence"].as_u64().unwrap() as u32, tag, &tx[at..at + 33], Some(der.as_bytes()));
    serde_json::to_string(&serde_json::json!({
        "tx_blob": hex::encode_upper(&signed),
        "hash": hex::encode_upper(sha512_half(&[b"TXN\0", &signed])),
    }))
    .map_err(err)
}

#[cfg(test)]
mod tests;
