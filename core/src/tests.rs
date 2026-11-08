use super::*;
use defuse_wallet_sdk::WalletBuilder;
use defuse_wallet_webauthn::{WalletWebauthn, mock::MockWalletWebauthnSigner, p256::P256, webauthn::IgnoreUserVerification};
use p256::ecdsa::SigningKey;
use sha2::{Digest, Sha256};

// Vectors from near/intents at 32cd1f9: contracts/wallet/src/message.rs
// (json_hash) and contracts/wallet/signatures/webauthn/src/p256.rs (verify_ok).
const MSG: &str = r#"{"nonce":2845491008,"request":{"external":[{"actions":[{"action":"function_call","payload":{"args":"eyJyZXF1ZXN0Ijp7InBheWxvYWRfdjIiOnsiRWNkc2EiOiIwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwIn0sImRvbWFpbl9pZCI6MCwicGF0aCI6IiJ9fQ==","deposit":"1","function_name":"sign"}}],"receiver_id":"v1.signer"}]},"chain_id":"mainnet","signer_id":"0se5eba21e8f191e1880e453794bc551dfa50a3419","created_at":"2026-07-07T11:13:29Z","timeout_secs":3600}"#;
const PK: &str = "02c0cad83d84b6bf228366971ddcb1738b9af862c64446a9feea85964743a5b390";
const PROOF: &str = r#"{"authenticator_data":"SZYN5YgOjGh0NBcPZHZgW4_krrmihjLHmVzzuoMdl2MdAAAAAA==","client_data_json":"{\"type\":\"webauthn.get\",\"challenge\":\"fIVgtROAVRZ2pU2sSdzB4AmvMCBVbN6OHiFyeC_9Z3U\",\"origin\":\"http://localhost:5173\",\"crossOrigin\":false}","signature":"p256:2S9WKwvKY7zgLaC9ihYDMQmbGtCvqqYy6RvvpgqRs7VXBR3GKXke4LKLNKyvtWRRB3dmU3awuCh2EYY7Sfk34qmh"}"#;

#[test]
fn hash_matches_wallet_vector() {
    assert_eq!(request_hash(MSG).unwrap(), "7c8560b51380551676a54dac49dcc1e009af3020556cde8e1e2172782ffd6775");
}

#[test]
fn normalized_request_hashes_the_same() {
    let n = normalize_request(MSG).unwrap();
    assert_eq!(request_hash(&n).unwrap(), request_hash(MSG).unwrap());
}

#[test]
fn wallet_vector_proof_verifies() {
    assert!(verify_proof(PK, MSG, PROOF).unwrap());
    // A different message must fail.
    let other = MSG.replace("2845491008", "2845491009");
    assert!(!verify_proof(PK, &other, PROOF).unwrap());
}

#[test]
fn account_id_matches_sdk() {
    let key = SigningKey::from_slice(&[7u8; 32]).unwrap();
    let code = [9u8; 32];
    let ext: defuse_wallet::AccountId = "ext.testnet".parse().unwrap();
    let w = WalletBuilder::new()
        .extension(ext.clone())
        .build::<WalletWebauthn<P256, IgnoreUserVerification>, _>(GlobalContractId::CodeHash(code), MockWalletWebauthnSigner::new(key.clone()));
    let sec1 = hex::encode(key.verifying_key().to_sec1_point(false).as_bytes());
    assert_eq!(wallet_account_id(&hex::encode(code), &sec1, ext.as_str()).unwrap(), w.account_id().to_string());
}

/// What a real authenticator does: DER-sign sha256(authData || sha256(clientDataJSON)).
fn assert_with(key: &SigningKey, msg: &str, high_s: bool) -> String {
    use p256::ecdsa::signature::Signer;
    let challenge = hex::decode(request_hash(msg).unwrap()).unwrap();
    let b64 = defuse_crypto_b64url(&challenge);
    let cdj = format!(r#"{{"type":"webauthn.get","challenge":"{b64}","origin":"https://example.test","crossOrigin":false}}"#);
    let auth = [[0x11u8; 32].as_slice(), &[0x05, 0, 0, 0, 1]].concat();
    let signed = [auth.as_slice(), Sha256::digest(cdj.as_bytes()).as_slice()].concat();
    let sig: Signature = key.sign(&signed);
    let sig = sig.normalize_s();
    let sig = if high_s {
        // Flip to high S: s' = n - s. Still a valid ECDSA signature.
        let (r, s) = sig.split_scalars();
        Signature::from_scalars(r, -*s).unwrap()
    } else {
        sig
    };
    make_proof(&hex::encode(auth), &cdj, &hex::encode(sig.to_der().as_bytes())).unwrap()
}

fn defuse_crypto_b64url(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..=c.len() {
            out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

#[test]
fn make_proof_verifies_and_normalizes_high_s() {
    let key = SigningKey::from_slice(&[3u8; 32]).unwrap();
    let pk = hex::encode(key.verifying_key().to_sec1_point(false).as_bytes());
    for high in [false, true] {
        let proof = assert_with(&key, MSG, high);
        assert!(verify_proof(&pk, MSG, &proof).unwrap(), "high_s={high}");
    }
}

#[test]
fn bad_inputs_rejected() {
    assert!(compress_public_key("04deadbeef").is_err());
    assert!(make_proof("00", "{}", "3000").is_err());
    assert!(request_hash("{}").is_err());
}

#[test]
fn evm_address_matches_testnet_wallet() {
    // derived_public_key(path "evm-84532", predecessor the testnet wallet
    // 0s978868634534dd8d2a78432b2c9121dbc9b5cada) on v1.signer-prod.testnet,
    // whose MPC signature recovered to this address on 8 November 2026.
    let k = "secp256k1:4WRNPg3KHo6enGj54r82i7vWWdffstSiaXxXNodPyrYNtPsxXgQtJ4o1vvgY3tTA9XjPM6BhNExvYXqG8RoDiaGP";
    assert_eq!(evm_address(k).unwrap(), "0xe55af65fe8f321019d624fa88ff38a611ee6ee67");
    assert!(evm_address("ed25519:abc").is_err());
}

#[test]
fn call_message_round_trips_and_describes() {
    let m = call_message("testnet", "0s978868634534dd8d2a78432b2c9121dbc9b5cada", 7, 1_762_560_000, "ext.testnet", "revoke_session", r#"{"key_id":"agent-1"}"#, "0", 20).unwrap();
    // Parses back as the wallet's type and hashes stably.
    assert_eq!(request_hash(&normalize_request(&m).unwrap()).unwrap(), request_hash(&m).unwrap());
    let d: serde_json::Value = serde_json::from_str(&describe_message(&m).unwrap()).unwrap();
    assert_eq!(d["external"][0]["receiver_id"], "ext.testnet");
    assert_eq!(d["external"][0]["actions"][0]["method"], "revoke_session");
    assert_eq!(d["external"][0]["actions"][0]["args"]["key_id"], "agent-1");
    assert_eq!(d["external"][0]["actions"][0]["gas_tgas"], 20);
    assert_eq!(d["internal"].as_array().unwrap().len(), 0);
}

#[test]
fn describe_shows_wallet_ops() {
    // A message that removes an extension must show it, never hide it.
    use defuse_wallet::{Request, Timestamp, WalletOp};
    let msg = RequestMessage {
        pay_for_gas: false,
        chain_id: "testnet".into(),
        signer_id: "0s978868634534dd8d2a78432b2c9121dbc9b5cada".parse().unwrap(),
        nonce: 1,
        created_at: Timestamp::from_secs(1_762_560_000).unwrap(),
        timeout: core::time::Duration::from_secs(600),
        request: Request { internal: vec![WalletOp::RemoveExtension { account_id: "ext.testnet".parse().unwrap() }], external: vec![] },
    };
    let d = describe_message(&serde_json::to_string(&msg).unwrap()).unwrap();
    assert!(d.contains(r#""op":"remove_extension""#) && d.contains("ext.testnet"), "{d}");
}

// The same transaction as the extension's `evm_signing_hash_matches_cast`
// (reference computed with `cast mktx` and `cast keccak`).
fn cast_tx() -> String {
    let data = format!("0x095ea7b3{:0>64}{:064x}", "3333333333333333333333333333333333333333", 1_000_000u128);
    format!(r#"{{"chain_id":84532,"nonce":7,"max_priority_fee_per_gas":"1000000","max_fee_per_gas":"2000000000","gas_limit":210000,"to":"0x2222222222222222222222222222222222222222","value":"5","data":"{data}"}}"#)
}

#[test]
fn evm_hash_matches_cast_and_decodes() {
    let d: serde_json::Value = serde_json::from_str(&describe_evm_tx(&cast_tx()).unwrap()).unwrap();
    assert_eq!(d["hash"], "743d877c584f6b318e1199aa38eecfdb73d463badffe53ec78c0a67ff0435ccd");
    assert_eq!(d["call"]["kind"], "erc20_approve");
    assert_eq!(d["call"]["spender"], "0x3333333333333333333333333333333333333333");
    assert_eq!(d["call"]["amount"], "1000000");
}

#[test]
fn aave_supply_decodes() {
    // supply(USDC, 20 USDC, 0x48f0…, 0), as broadcast on Base Sepolia.
    let data = format!("0x617ba037{:0>64}{:064x}{:0>64}{:064x}", "ba50cd2a20f6da35d788639e581bca8d0b5d4d5f", 20_000_000u128, "48f0deef2c7e15ea9b4398a6875b43b5ab6ea81e", 0);
    let tx = format!(r#"{{"chain_id":84532,"nonce":1,"max_priority_fee_per_gas":"1","max_fee_per_gas":"2","gas_limit":300000,"to":"0x8bab6d1b75f19e9ed9fce8b9bd338844ff79ae27","value":"0","data":"{data}"}}"#);
    let d: serde_json::Value = serde_json::from_str(&describe_evm_tx(&tx).unwrap()).unwrap();
    assert_eq!(d["call"]["kind"], "aave_supply");
    assert_eq!(d["call"]["asset"], "0xba50cd2a20f6da35d788639e581bca8d0b5d4d5f");
    assert_eq!(d["call"]["amount"], "20000000");
    assert_eq!(d["call"]["on_behalf_of"], "0x48f0deef2c7e15ea9b4398a6875b43b5ab6ea81e");
    // Unknown functions are shown as unknown.
    let other = tx.replace("0x617ba037", "0xdeadbeef");
    assert!(describe_evm_tx(&other).unwrap().contains("unknown_call"));
}

#[test]
fn owner_sign_message_carries_our_hash() {
    let m = owner_evm_sign_message("testnet", "0s3465557e026f471c0e84898f799c8032f356c77a", 9, 1_762_560_000, "v1.signer-prod.testnet", &cast_tx()).unwrap();
    let d: serde_json::Value = serde_json::from_str(&describe_message(&m).unwrap()).unwrap();
    let a = &d["external"][0]["actions"][0];
    assert_eq!(d["external"][0]["receiver_id"], "v1.signer-prod.testnet");
    assert_eq!(a["method"], "sign");
    assert_eq!(a["deposit_yocto"], "1");
    assert_eq!(a["args"]["request"]["path"], "evm");
    assert_eq!(a["args"]["request"]["payload_v2"]["Ecdsa"], "743d877c584f6b318e1199aa38eecfdb73d463badffe53ec78c0a67ff0435ccd");
}

// Reference from viem 2.57.8: privateKeyToAccount(0x11…11).signTransaction of
// this exact transaction (a 5 USDC transfer to 0x…dEaD on Base Sepolia).
const VIEM_FROM: &str = "0x19e7e376e7c213b7e7e7e46cc70a5dd086daff2a";
const VIEM_RAW: &str = "0x02f8b183014a3403830f4240847735940082ea6094ba50cd2a20f6da35d788639e581bca8d0b5d4d5f80b844a9059cbb000000000000000000000000000000000000000000000000000000000000dead00000000000000000000000000000000000000000000000000000000004c4b40c001a0262260a81736ec3e7cc75c49f74a761c7392550aa9d936e4944a08cc3b3ec981a03b5323938051958d7ca6f6a1e4ec89ae9363aac9a2933708659ac42acc8449a3";

fn send_tx() -> String {
    evm_send_tx(84532, 3, "1000000", "2000000000", 60000, "0xba50cd2a20f6da35d788639e581bca8d0b5d4d5f", "0x000000000000000000000000000000000000dEaD", "5000000").unwrap()
}

/// Signs as the MPC signer would answer: R as a compressed point, S, and
/// the recovery id. `high_s` flips S to the other valid form.
fn mpc_sign(tx: &str, high_s: bool) -> String {
    use k256::ecdsa::SigningKey;
    let key = SigningKey::from_slice(&[0x11; 32]).unwrap();
    let hash: [u8; 32] = hex::decode(serde_json::from_str::<serde_json::Value>(&describe_evm_tx(tx).unwrap()).unwrap()["hash"].as_str().unwrap()).unwrap().try_into().unwrap();
    let (sig, recid) = key.sign_prehash_recoverable(&hash).unwrap();
    let (r, mut s) = (sig.r().to_bytes().to_vec(), sig.s().to_bytes().to_vec());
    let mut y = recid.is_y_odd() as u8;
    if high_s {
        let flipped = k256::ecdsa::Signature::from_scalars(sig.r().to_bytes(), (-*sig.s()).to_bytes()).unwrap();
        s = flipped.s().to_bytes().to_vec();
        y ^= 1;
    }
    serde_json::json!({
        "big_r": { "affine_point": format!("{:02x}{}", 2 + y, hex::encode(&r)) },
        "s": { "scalar": hex::encode(&s) },
        "recovery_id": y,
    })
    .to_string()
}

#[test]
fn signed_tx_matches_viem() {
    let tx = send_tx();
    for high in [false, true] {
        let out: serde_json::Value = serde_json::from_str(&evm_signed_tx(&tx, &mpc_sign(&tx, high)).unwrap()).unwrap();
        assert_eq!(out["from"], VIEM_FROM, "high_s={high}");
        assert_eq!(out["raw"], VIEM_RAW, "high_s={high}");
    }
}

#[test]
fn signed_tx_for_another_tx_recovers_someone_else() {
    // A signature over a different transaction recovers to a different
    // address, which the page then refuses to broadcast.
    let tx = send_tx();
    let other = tx.replace("\"nonce\":3", "\"nonce\":4");
    let out: serde_json::Value = serde_json::from_str(&evm_signed_tx(&other, &mpc_sign(&tx, false)).unwrap()).unwrap();
    assert_ne!(out["from"], VIEM_FROM);
}

#[test]
fn native_send_has_value_and_no_data() {
    let t: serde_json::Value = serde_json::from_str(&evm_send_tx(84532, 0, "1", "2", 21000, "", "0x000000000000000000000000000000000000dEaD", "1000").unwrap()).unwrap();
    assert_eq!(t["to"], "0x000000000000000000000000000000000000dead");
    assert_eq!(t["value"], "1000");
    assert_eq!(t["data"], "0x");
}
