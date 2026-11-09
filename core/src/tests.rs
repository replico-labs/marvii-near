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
    let (sig, recid) = key.sign_prehash_recoverable(&hash);
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

// Reference addresses from each chain's own library (bitcoinjs-lib 6.1.7,
// bchaddrjs 0.5.2, tronweb 6.0.0, ripple-keypairs 2.0.0, @solana/web3.js
// 1.98.0; Dogecoin, Zcash and Cardano by their version bytes / CIP-19 with
// bs58check and bech32), for secp256k1 key 0x11…11 and ed25519 seed 0x22…22.
#[test]
fn chain_addresses_match_reference_libraries() {
    let xy = hex::decode("4f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa385b6b1b8ead809ca67454d9683fcf2ba03456d6fe2c4abe2b07f0fbdbb2f1c1").unwrap();
    let k = format!("secp256k1:{}", bs58::encode(xy).into_string());
    let e = format!("ed25519:{}", bs58::encode(hex::decode("a09aa5f47a6759802ff955f8dc2d2a14a5c99d23be97f864127ff9383455a4f0").unwrap()).into_string());
    for (chain, key, want) in [
        ("bitcoin", &k, "tb1ql3e9pgs3mmwuwrh95fecme0s0qtn28804khrk8"),
        ("litecoin", &k, "tltc1ql3e9pgs3mmwuwrh95fecme0s0qtn2880v74axw"),
        ("dogecoin", &k, "nsCyVMbwVcbNFQNHBmEtyUsx9YCzMC8mub"),
        ("bitcoin-cash", &k, "bchtest:qr78y59zz80dm3cwuk388r097pupwdguaugjl6473v"),
        ("zcash", &k, "tmYjAZFpvdDXTaJrq2WikAntitBNJJo9VSo"),
        ("tron", &k, "TCLBgkbfVkJroVBJVqBEsxtPNQEQMTQCLQ"),
        ("xrp", &k, "rQrFNnvPGNNMqRcVRMbtBK3hpYaPzoa8K9"),
        ("solana", &e, "Bow1CGKGDB9mNxeWdw85E2aCthQ1oZX4oFEe7fYT17ew"),
        ("cardano", &e, "addr_test1vr523hvdkxflk0cv9swltju5vgxds6ly8e8q25ulceutrdgyneq9q"),
    ] {
        assert_eq!(chain_address(chain, key).unwrap(), want, "{chain}");
    }
    assert!(chain_address("solana", &k).is_err());
    assert!(chain_address("bitcoin", &e).is_err());
}

// Reference from @solana/web3.js 1.98.0: SystemProgram.transfer of 1234567
// lamports from Keypair.fromSeed(0x22…22) to Keypair.fromSeed(0x33…33),
// blockhash EETubP5A…hq1k, signed by the 0x22 key.
const SOL_FROM: &str = "Bow1CGKGDB9mNxeWdw85E2aCthQ1oZX4oFEe7fYT17ew";
const SOL_TO: &str = "2btLJAAb1S3x6hZYdVyAePjqtQYi2ZBSRGy4569RZu8h";
const SOL_MSG: &str = "01000103a09aa5f47a6759802ff955f8dc2d2a14a5c99d23be97f864127ff9383455a4f017cb79fb2b4120f2b1ec65e4198d6e08b28e813feb01e4a400839b85e18080ce0000000000000000000000000000000000000000000000000000000000000000c49ae77603782054f17a9decea43b444eba0edb12c6f1d31c6e0e4a84bf052eb01020200010c0200000087d6120000000000";
const SOL_TX: &str = "AYbdviGp8Zhgydk7ewscUtxkyeRtnP+J/e0cuZE0eCCLcQiap28MDIvqPKFeBbLNgNP1x/AWR5R+kI/2GGx8LAoBAAEDoJql9HpnWYAv+VX43C0qFKXJnSO+l/hkEn/5ODRVpPAXy3n7K0Eg8rHsZeQZjW4Iso6BP+sB5KQAg5uF4YCAzgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAxJrndgN4IFTxep3s6kO0ROug7bEsbx0xxuDkqEvwUusBAgIAAQwCAAAAh9YSAAAAAAA=";

#[test]
fn solana_message_and_tx_match_web3js() {
    let m = solana_transfer_message(SOL_FROM, SOL_TO, "1234567", "EETubP5AKHgjPAhzPAFcb8BAY1hMH639CWCFTqi3hq1k").unwrap();
    assert_eq!(m, SOL_MSG);
    let d: serde_json::Value = serde_json::from_str(&describe_solana_message(&m).unwrap()).unwrap();
    assert_eq!((d["from"].as_str().unwrap(), d["to"].as_str().unwrap(), d["lamports"].as_str().unwrap()), (SOL_FROM, SOL_TO, "1234567"));
    // Sign as the MPC signer would answer, with the 0x22 key.
    use ed25519_dalek::{Signer, SigningKey};
    let sig = SigningKey::from_bytes(&[0x22; 32]).sign(&hex::decode(&m).unwrap());
    let resp = serde_json::json!({ "scheme": "Ed25519", "signature": sig.to_bytes().to_vec() }).to_string();
    assert_eq!(solana_signed_tx(&m, &resp, SOL_FROM).unwrap(), SOL_TX);
    // The same signature for another account, or another message, is refused.
    assert!(solana_signed_tx(&m, &resp, SOL_TO).is_err());
    let other = solana_transfer_message(SOL_FROM, SOL_TO, "1234568", "EETubP5AKHgjPAhzPAFcb8BAY1hMH639CWCFTqi3hq1k").unwrap();
    assert!(solana_signed_tx(&other, &resp, SOL_FROM).is_err());
}

#[test]
fn solana_describe_refuses_other_messages() {
    let mut m = hex::decode(SOL_MSG).unwrap();
    let n = m.len();
    m[n - 12] = 3; // another System instruction
    assert!(describe_solana_message(&hex::encode(m)).is_err());
}

#[test]
fn owner_eddsa_message_uses_the_chain_path() {
    let m = owner_eddsa_sign_message("testnet", "0s3465557e026f471c0e84898f799c8032f356c77a", 1, 1_762_560_000, "v1.signer-prod.testnet", "solana", SOL_MSG).unwrap();
    let d: serde_json::Value = serde_json::from_str(&describe_message(&m).unwrap()).unwrap();
    let r = &d["external"][0]["actions"][0]["args"]["request"];
    assert_eq!((r["path"].as_str().unwrap(), r["domain_id"].as_u64().unwrap()), ("solana", 1));
    assert_eq!(r["payload_v2"]["Eddsa"], SOL_MSG);
    assert!(owner_eddsa_sign_message("testnet", "0s3465557e026f471c0e84898f799c8032f356c77a", 1, 1, "v1.signer-prod.testnet", "bitcoin", SOL_MSG).is_err());
}

// Reference from bitcoinjs-lib 6.1.7 (Psbt, P2WPKH, key 0x11…11, sequence
// 0xfffffffd): 60000 + 25000 sat in; 70000 to tb1qw508…pjzsx, 14000 change.
const BTC_FROM: &str = "tb1ql3e9pgs3mmwuwrh95fecme0s0qtn28804khrk8";
const BTC_RAW: &str = "02000000000102aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa0100000000fdffffffbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb0000000000fdffffff027011010000000000160014751e76e8199196d454941c45d1b3a323f1433bd6b036000000000000160014fc7250a211deddc70ee5a2738de5f07817351cef02483045022100f5ce32c5422a6b89f177ac054918e58f6700311f6d37f59fb255a1e037c3791b02203c67b7bf330bbcd7bf862dd5ebbd14b210022111485e063913f24bbf7a95c4240121034f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa02483045022100c774a514e1d118bdc61dd000eb7a80c4d19aedfcf47cc68505c431662d1652fd0220255df0ef84d1fb2675d11aec7e43cbc105a30b21b1e5af5cc3079b2b9187dded0121034f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa00000000";
const BTC_TXID: &str = "e6d28e9d0d114c294fcc6eba8b105a36a340d75bd88749ff3fdfd3e611958925";

fn btc_reference_plan() -> String {
    let inputs = serde_json::json!([{ "txid": "aa".repeat(32), "vout": 1, "value": 60000 }, { "txid": "bb".repeat(32), "vout": 0, "value": 25000 }]);
    let outputs = vec![BtcOutput { address: "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx".into(), value: 70000 }, BtcOutput { address: BTC_FROM.into(), value: 14000 }];
    let inputs: Vec<BtcInput> = serde_json::from_value(inputs).unwrap();
    let sighashes = btc_sighashes(BTC_FROM, &inputs, &outputs).unwrap();
    serde_json::to_string(&BtcPlan { from: BTC_FROM.into(), inputs, outputs, fee: 1000, sighashes }).unwrap()
}

/// Signs each hash as the MPC signer would answer (k256 is RFC 6979, like
/// bitcoinjs), in reverse order to check signatures are matched by content.
fn btc_mpc_sigs(plan: &str) -> String {
    use k256::ecdsa::SigningKey;
    let key = SigningKey::from_slice(&[0x11; 32]).unwrap();
    let p: BtcPlan = serde_json::from_str(plan).unwrap();
    let sigs: Vec<_> = p.sighashes.iter().rev().map(|h| {
        let (sig, recid) = key.sign_prehash_recoverable(&hex::decode(h).unwrap());
        serde_json::json!({ "big_r": { "affine_point": format!("{:02x}{}", 2 + recid.is_y_odd() as u8, hex::encode(sig.r().to_bytes())) }, "s": { "scalar": hex::encode(sig.s().to_bytes()) }, "recovery_id": recid.to_byte() })
    }).collect();
    serde_json::to_string(&sigs).unwrap()
}

#[test]
fn bitcoin_tx_matches_bitcoinjs() {
    let plan = btc_reference_plan();
    let out: serde_json::Value = serde_json::from_str(&bitcoin_signed_tx(&plan, &btc_mpc_sigs(&plan)).unwrap()).unwrap();
    assert_eq!(out["raw"], BTC_RAW);
    assert_eq!(out["txid"], BTC_TXID);
}

#[test]
fn bitcoin_plan_selects_and_describes() {
    let utxos = serde_json::json!([{ "txid": "aa".repeat(32), "vout": 1, "value": 60000 }, { "txid": "bb".repeat(32), "vout": 0, "value": 25000 }, { "txid": "cc".repeat(32), "vout": 2, "value": 1000 }]).to_string();
    let plan = bitcoin_plan(BTC_FROM, "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx", "70000", &utxos, 2).unwrap();
    let p: BtcPlan = serde_json::from_str(&plan).unwrap();
    assert_eq!(p.inputs.len(), 2, "largest coins first");
    assert_eq!(p.outputs[1].address, BTC_FROM, "change back to the account");
    let d: serde_json::Value = serde_json::from_str(&describe_bitcoin_plan(&plan).unwrap()).unwrap();
    assert_eq!(d["pays"][0]["sat"], "70000");
    assert_eq!(d["pays"].as_array().unwrap().len(), 1, "change isn't shown as a payment");
    // A tampered plan (different recipient, same hashes) is refused.
    let tampered = plan.replace("tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx", "tb1ql3e9pgs3mmwuwrh95fecme0s0qtn28804khrk8");
    assert!(describe_bitcoin_plan(&tampered).is_err());
    assert!(bitcoin_plan(BTC_FROM, "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx", "90000", &utxos, 2).is_err(), "not enough");
}

#[test]
fn bitcoin_signatures_from_another_key_are_refused() {
    let plan = btc_reference_plan();
    let p: BtcPlan = serde_json::from_str(&plan).unwrap();
    let other = k256::ecdsa::SigningKey::from_slice(&[0x12; 32]).unwrap();
    let sigs: Vec<_> = p.sighashes.iter().map(|h| {
        let (sig, recid) = other.sign_prehash_recoverable(&hex::decode(h).unwrap());
        serde_json::json!({ "big_r": { "affine_point": format!("{:02x}{}", 2 + recid.is_y_odd() as u8, hex::encode(sig.r().to_bytes())) }, "s": { "scalar": hex::encode(sig.s().to_bytes()) }, "recovery_id": recid.to_byte() })
    }).collect();
    assert!(bitcoin_signed_tx(&plan, &serde_json::to_string(&sigs).unwrap()).is_err());
}

// Reference from tronweb 6.0.0 (txJsonToPb / txPbToRawDataHex / txPbToTxID /
// crypto.signTransaction) for 1234567 sun from key 0x11…11 to key 0x33…33's
// address.
const TRON_FROM: &str = "TCLBgkbfVkJroVBJVqBEsxtPNQEQMTQCLQ";
const TRON_TO: &str = "TJRabPrwbZy45sbavfcjinPJC18kjpRTv8";
const TRON_RAW: &str = "0a02a1b22208010203040506070840e0889a99a6335a67080112630a2d747970652e676f6f676c65617069732e636f6d2f70726f746f636f6c2e5472616e73666572436f6e747261637412320a154119e7e376e7c213b7e7e7e46cc70a5dd086daff2a1215415cbdd86a2fa8dc4bddd8a8f69dba48572eec07fb1887ad4b7080b49699a633";
const TRON_TXID: &str = "15e0f5dd80e24a9f1ba17c47322437a76b4f51be2498baba756687565b83f0c3";
const TRON_SIG: &str = "c5e2c7e2a5ab1738324d076bf57673b3c9b92bc7daceee0e9334cc5d3a675fb606400f5fcfaeba9c1ab8173d323dd74882b86dd77f09da512f5c9466bda73ddd1b";

#[test]
fn tron_raw_and_signature_match_tronweb() {
    let r: serde_json::Value = serde_json::from_str(&tron_transfer_raw(TRON_FROM, TRON_TO, "1234567", "a1b2", "0102030405060708", 1762600060000, 1762600000000).unwrap()).unwrap();
    assert_eq!(r["raw_hex"], TRON_RAW);
    assert_eq!(r["txid"], TRON_TXID);
    let d: serde_json::Value = serde_json::from_str(&describe_tron_raw(TRON_RAW).unwrap()).unwrap();
    assert_eq!((d["from"].as_str().unwrap(), d["to"].as_str().unwrap(), d["sun"].as_str().unwrap()), (TRON_FROM, TRON_TO, "1234567"));
    let key = k256::ecdsa::SigningKey::from_slice(&[0x11; 32]).unwrap();
    let (sig, recid) = key.sign_prehash_recoverable(&hex::decode(TRON_TXID).unwrap());
    let resp = serde_json::json!({ "big_r": { "affine_point": format!("{:02x}{}", 2 + recid.is_y_odd() as u8, hex::encode(sig.r().to_bytes())) }, "s": { "scalar": hex::encode(sig.s().to_bytes()) }, "recovery_id": recid.to_byte() }).to_string();
    let out: serde_json::Value = serde_json::from_str(&tron_signed_tx(TRON_RAW, &resp, TRON_FROM).unwrap()).unwrap();
    assert_eq!(out["signature"], TRON_SIG);
    assert!(tron_signed_tx(TRON_RAW, &resp, TRON_TO).is_err());
}

#[test]
fn tron_describe_refuses_tampering() {
    // Change the amount byte inside the contract: re-encoding no longer matches the decode.
    // The contract-type field (`08 01`, after the contract tag `5a 67`) set
    // to another contract type: the strict re-encode no longer matches.
    let mut raw = hex::decode(TRON_RAW).unwrap();
    let pos = raw.windows(4).position(|w| w == [0x5a, 0x67, 0x08, 0x01]).unwrap();
    raw[pos + 3] = 2;
    assert!(describe_tron_raw(&hex::encode(&raw)).is_err());
    // An extra field appended (e.g. a fee limit) is refused too.
    let mut extra = hex::decode(TRON_RAW).unwrap();
    extra.extend([0x90, 0x01, 0x01]);
    assert!(describe_tron_raw(&hex::encode(extra)).is_err());
}

// Reference from xrpl.js 4.1.0 / ripple-binary-codec 2.1.0 (encode,
// encodeForSigning, hashes.hashSignedTx) with ripple-keypairs 2.0.0 `sign`:
// 1234567 drops, fee 12, sequence 7, last ledger 22176800, from key 0x11…11
// to key 0x33…33's address; the second also has destination tag 4242.
const XRP_FROM: &str = "rQrFNnvPGNNMqRcVRMbtBK3hpYaPzoa8K9";
const XRP_TO: &str = "raSyAhSugW1h15KbK3cuxJXMpuk6pGKFR5";
const XRP_KEY_XY: &str = "4f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa385b6b1b8ead809ca67454d9683fcf2ba03456d6fe2c4abe2b07f0fbdbb2f1c1";
const XRP_VECTORS: [(f64, &str, &str, &str); 2] = [
    (-1.0,
     "1200002400000007201B0152642061400000000012D68768400000000000000C7321034F355BDCB7CC0AF728EF3CCEB9615D90684BB5B2CA5F859AB0F0B704075871AA74463044022075BD4AA86F098EF0BD05D513B843A0E4A30CC5E8A2BA6C9012392BA0AB2C7C1D02202FA45C8FD8E413B6F2A1DE9F6D1895B8D7B47C8DBF7A925198DE8EE8F32F803E8114FC7250A211DEDDC70EE5A2738DE5F07817351CEF83143BC28D6D92D9073FB5E3ADF481795EAF446BCEED",
     "1200002400000007201B0152642061400000000012D68768400000000000000C7321034F355BDCB7CC0AF728EF3CCEB9615D90684BB5B2CA5F859AB0F0B704075871AA8114FC7250A211DEDDC70EE5A2738DE5F07817351CEF83143BC28D6D92D9073FB5E3ADF481795EAF446BCEED",
     "13349C7DCEB9479C18DEF96F446675714866A53F1C36BE92EF99BD801DAA172A"),
    (4242.0,
     "12000024000000072E00001092201B0152642061400000000012D68768400000000000000C7321034F355BDCB7CC0AF728EF3CCEB9615D90684BB5B2CA5F859AB0F0B704075871AA7446304402201AA3E2DEDF9C7E4E2A70F1607623B21A9BFE3493B3F4EB4DB0FC982C29A3EEE7022051AB8EDA2A742B50B8F009C90AB857FDE3A23CF71EF26D173DFCA2CB221CCA9B8114FC7250A211DEDDC70EE5A2738DE5F07817351CEF83143BC28D6D92D9073FB5E3ADF481795EAF446BCEED",
     "12000024000000072E00001092201B0152642061400000000012D68768400000000000000C7321034F355BDCB7CC0AF728EF3CCEB9615D90684BB5B2CA5F859AB0F0B704075871AA8114FC7250A211DEDDC70EE5A2738DE5F07817351CEF83143BC28D6D92D9073FB5E3ADF481795EAF446BCEED",
     "7E53D6B41CFC5F031AC294CA2D0D5EC3737251DFA6097616E13CD24C11AFDE7D"),
];

fn xrp_mpc_sign(hash_hex: &str, key: [u8; 32], high_s: bool) -> String {
    let key = k256::ecdsa::SigningKey::from_slice(&key).unwrap();
    let (sig, recid) = key.sign_prehash_recoverable(&hex::decode(hash_hex).unwrap());
    let mut y = recid.is_y_odd() as u8;
    let mut s = sig.s().to_bytes();
    if high_s {
        s = (-*sig.s()).to_bytes();
        y ^= 1;
    }
    serde_json::json!({ "big_r": { "affine_point": format!("{:02x}{}", 2 + recid.is_y_odd() as u8, hex::encode(sig.r().to_bytes())) }, "s": { "scalar": hex::encode(s) }, "recovery_id": y }).to_string()
}

#[test]
fn xrp_payment_matches_xrpl_js() {
    let key = format!("secp256k1:{}", bs58::encode(hex::decode(XRP_KEY_XY).unwrap()).into_string());
    for (tag, signed, unsigned, hash) in XRP_VECTORS {
        let p: serde_json::Value = serde_json::from_str(&xrp_payment(XRP_FROM, &key, XRP_TO, "1234567", "12", 7, 22176800, tag).unwrap()).unwrap();
        assert_eq!(p["tx_hex"].as_str().unwrap().to_uppercase(), unsigned);
        let d: serde_json::Value = serde_json::from_str(&describe_xrp_tx(p["tx_hex"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!((d["from"].as_str().unwrap(), d["to"].as_str().unwrap(), d["drops"].as_str().unwrap(), d["fee_drops"].as_str().unwrap()), (XRP_FROM, XRP_TO, "1234567", "12"));
        assert_eq!(d["destination_tag"].as_u64(), if tag < 0.0 { None } else { Some(tag as u64) });
        // A high-S signature from the signer is normalised; both give xrpl.js's blob.
        for high_s in [false, true] {
            let sig = xrp_mpc_sign(p["signing_hash"].as_str().unwrap(), [0x11; 32], high_s);
            let out: serde_json::Value = serde_json::from_str(&xrp_signed_tx(p["tx_hex"].as_str().unwrap(), &sig, XRP_FROM).unwrap()).unwrap();
            assert_eq!(out["tx_blob"], signed);
            assert_eq!(out["hash"], hash);
        }
        // Another key's signature, or the wrong account, is refused.
        let other = xrp_mpc_sign(p["signing_hash"].as_str().unwrap(), [0x33; 32], false);
        assert!(xrp_signed_tx(p["tx_hex"].as_str().unwrap(), &other, XRP_FROM).is_err());
        let sig = xrp_mpc_sign(p["signing_hash"].as_str().unwrap(), [0x11; 32], false);
        assert!(xrp_signed_tx(p["tx_hex"].as_str().unwrap(), &sig, XRP_TO).is_err());
    }
}

#[test]
fn xrp_refuses_bad_inputs_and_tampering() {
    let key = format!("secp256k1:{}", bs58::encode(hex::decode(XRP_KEY_XY).unwrap()).into_string());
    // Another account's key, a fee over the cap, and sending to yourself.
    assert!(xrp_payment(XRP_TO, &key, XRP_FROM, "1", "12", 1, 1, -1.0).is_err());
    assert!(xrp_payment(XRP_FROM, &key, XRP_TO, "1", "100001", 1, 1, -1.0).is_err());
    assert!(xrp_payment(XRP_FROM, &key, XRP_FROM, "1", "12", 1, 1, -1.0).is_err());
    let unsigned = hex::decode(XRP_VECTORS[0].2).unwrap();
    // Flags inserted, a token (non-XRP) amount, a field appended: all refused.
    let mut flags = unsigned.clone();
    flags.splice(3..3, [0x22, 0, 0, 0, 0]);
    assert!(describe_xrp_tx(&hex::encode(flags)).is_err());
    let mut iou = unsigned.clone();
    let a = iou.iter().position(|b| *b == 0x61).unwrap();
    iou[a + 1] = 0xd4;
    assert!(describe_xrp_tx(&hex::encode(iou)).is_err());
    let mut extra = unsigned.clone();
    extra.extend([0xf9]);
    assert!(describe_xrp_tx(&hex::encode(extra)).is_err());
}

// Reference from bitcoinjs-lib 6.1.7 with Litecoin testnet's parameters
// (bech32 "tltc", P2PKH 0x6f, P2SH 0x3a): key 0x11…11, 6,000,000 +
// 2,500,000 litoshis in; 7,000,000 to a tltc1 address, 300,000 to a Q…
// (P2SH) address, 1,190,000 change.
const LTC_FROM: &str = "tltc1ql3e9pgs3mmwuwrh95fecme0s0qtn2880v74axw";
const LTC_TO: &str = "tltc1qw508d6qejxtdg4y5r3zarvary0c5xw7klfsuq0";
const LTC_Q: &str = "QSpwqbSBqWfenXFv2Wg6zJj794QtwmrUQg";
const LTC_RAW: &str = "02000000000102aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa0100000000fdffffffbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb0000000000fdffffff03c0cf6a0000000000160014751e76e8199196d454941c45d1b3a323f1433bd6e09304000000000017a9144444444444444444444444444444444444444444877028120000000000160014fc7250a211deddc70ee5a2738de5f07817351cef02473044022036163b75ab6dfc1fa92144a2210f46c3e40d8462a58bd289a09eea9ee5673e90022059be2e4fbb35173854d5d67c12261e04efd84a10fdd9b81613915f42d6a530110121034f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa02483045022100f21d1b7cf3c92ec2ac0789fc8989685453d8807b8c207c6d8548ac4cb957334402200945ce7f3997e8b79ed0866bf2c0a98aba2eb01339734d3803ce66ce5cd487a00121034f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa00000000";
const LTC_TXID: &str = "22cdd9480b7895eb1789f706f5578d129ee7cefd151f8c43d8840de21bdbefe4";

#[test]
fn litecoin_tx_matches_bitcoinjs() {
    let inputs: Vec<BtcInput> = serde_json::from_value(serde_json::json!([{ "txid": "aa".repeat(32), "vout": 1, "value": 6000000 }, { "txid": "bb".repeat(32), "vout": 0, "value": 2500000 }])).unwrap();
    let outputs = vec![
        BtcOutput { address: LTC_TO.into(), value: 7000000 },
        BtcOutput { address: LTC_Q.into(), value: 300000 },
        BtcOutput { address: LTC_FROM.into(), value: 1190000 },
    ];
    let sighashes = btc_sighashes(LTC_FROM, &inputs, &outputs).unwrap();
    let plan = serde_json::to_string(&BtcPlan { from: LTC_FROM.into(), inputs, outputs, fee: 10000, sighashes }).unwrap();
    let d: serde_json::Value = serde_json::from_str(&describe_bitcoin_plan(&plan).unwrap()).unwrap();
    assert_eq!(d["chain"], "litecoin");
    assert_eq!(d["pays"].as_array().unwrap().len(), 2);
    let out: serde_json::Value = serde_json::from_str(&bitcoin_signed_tx(&plan, &btc_mpc_sigs(&plan)).unwrap()).unwrap();
    assert_eq!(out["raw"], LTC_RAW);
    assert_eq!(out["txid"], LTC_TXID);
}

#[test]
fn litecoin_and_bitcoin_addresses_dont_mix() {
    let utxos = serde_json::json!([{ "txid": "aa".repeat(32), "vout": 1, "value": 6000000 }]).to_string();
    // A Bitcoin address from a Litecoin account, and the other way round.
    assert!(bitcoin_plan(LTC_FROM, "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx", "1000000", &utxos, 1).is_err());
    assert!(bitcoin_plan(BTC_FROM, LTC_TO, "1000000", &utxos, 1).is_err());
    assert!(bitcoin_plan(BTC_FROM, LTC_Q, "1000000", &utxos, 1).is_err(), "Q… is Litecoin-only");
    // Litecoin's higher dust limit: 0.0005 LTC is refused, a Bitcoin-sized amount isn't.
    assert!(bitcoin_plan(LTC_FROM, LTC_TO, "50000", &utxos, 1).is_err());
    let plan = bitcoin_plan(LTC_FROM, LTC_TO, "1000000", &utxos, 1).unwrap();
    let d: serde_json::Value = serde_json::from_str(&describe_bitcoin_plan(&plan).unwrap()).unwrap();
    assert_eq!((d["chain"].as_str().unwrap(), d["pays"][0]["to"].as_str().unwrap()), ("litecoin", LTC_TO));
    let b = bitcoin_plan(BTC_FROM, "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx", "50000", &utxos, 1).unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&describe_bitcoin_plan(&b).unwrap()).unwrap()["chain"], "bitcoin");
}

#[test]
fn near_transfer_message_decodes_to_one_transfer() {
    let w = "0s3465557e026f471c0e84898f799c8032f356c77a";
    let m = transfer_message("testnet", w, 7, 1_762_600_000, "alice.marvii.testnet", "1500000000000000000000000").unwrap();
    let d: serde_json::Value = serde_json::from_str(&describe_message(&m).unwrap()).unwrap();
    assert_eq!(d["external"][0]["receiver_id"], "alice.marvii.testnet");
    assert_eq!(d["external"][0]["actions"][0]["kind"], "transfer");
    assert_eq!(d["external"][0]["actions"][0]["amount_yocto"], "1500000000000000000000000");
    assert_eq!(d["internal"].as_array().unwrap().len(), 0);
    assert!(transfer_message("testnet", w, 7, 1_762_600_000, w, "1").is_err(), "to yourself");
    assert!(transfer_message("testnet", w, 7, 1_762_600_000, "Not An Account", "1").is_err());
    assert!(transfer_message("testnet", w, 7, 1_762_600_000, "bob.testnet", "0").is_err());
}

// ---- Dogecoin, Bitcoin Cash, Zcash (P2PKH) and Cardano ----
// Key 0x11…11, as above: addresses from chain_addresses_match_reference_libraries.
const DOGE_FROM: &str = "nsCyVMbwVcbNFQNHBmEtyUsx9YCzMC8mub";
const BCH_FROM: &str = "bchtest:qr78y59zz80dm3cwuk388r097pupwdguaugjl6473v";
const ZEC_FROM: &str = "tmYjAZFpvdDXTaJrq2WikAntitBNJJo9VSo";
const ADA_FROM: &str = "addr_test1vr523hvdkxflk0cv9swltju5vgxds6ly8e8q25ulceutrdgyneq9q";

fn legacy_utxos() -> String {
    serde_json::json!([{ "txid": "aa".repeat(32), "vout": 1, "value": 600_000_000u64 }, { "txid": "bb".repeat(32), "vout": 0, "value": 250_000_000u64 }]).to_string()
}

#[test]
fn legacy_plans_sign_and_assemble() {
    for (chain, from, to) in [
        ("dogecoin", DOGE_FROM, "nsCyVMbwVcbNFQNHBmEtyUsx9YCzMC8mub"),
        ("bitcoin-cash", BCH_FROM, "bchtest:qr78y59zz80dm3cwuk388r097pupwdguaugjl6473v"),
        ("zcash", ZEC_FROM, "tmYjAZFpvdDXTaJrq2WikAntitBNJJo9VSo"),
    ] {
        let plan = legacy_plan(chain, from, to, "700000000", &legacy_utxos(), 1000, 0x7719_0ad9, 3_900_000).unwrap();
        let d: serde_json::Value = serde_json::from_str(&describe_legacy_plan(&plan).unwrap()).unwrap();
        assert_eq!(d["chain"], chain);
        assert_eq!(d["inputs"], 2);
        let out: serde_json::Value = serde_json::from_str(&legacy_signed_tx(&plan, &btc_mpc_sigs_any(&plan, [0x11; 32])).unwrap()).unwrap();
        assert!(out["raw"].as_str().unwrap().len() > 400, "{chain}");
        // Another key's signatures are refused.
        assert!(legacy_signed_tx(&plan, &btc_mpc_sigs_any(&plan, [0x33; 32])).is_err(), "{chain}");
        // Tampering with an output is caught.
        let tampered = plan.replace("\"value\":700000000", "\"value\":700000001");
        assert!(describe_legacy_plan(&tampered).is_err(), "{chain}");
        assert!(legacy_plan(chain, from, to, "900000000", &legacy_utxos(), 1000, 0x7719_0ad9, 3_900_000).is_err(), "not enough {chain}");
    }
    // Addresses don't cross chains.
    assert!(legacy_plan("dogecoin", DOGE_FROM, ZEC_FROM, "700000000", &legacy_utxos(), 1000, 0, 0).is_err());
    assert!(legacy_plan("zcash", ZEC_FROM, DOGE_FROM, "700000000", &legacy_utxos(), 1000, 1, 1).is_err());
    assert!(legacy_plan("bitcoin-cash", BCH_FROM, "bchtest:qr78y59zz80dm3cwuk388r097pupwdguaugjl6474v", "700000000", &legacy_utxos(), 1, 0, 0).is_err(), "bad checksum");
    // Zcash without a branch ID is refused.
    assert!(legacy_plan("zcash", ZEC_FROM, ZEC_FROM, "700000000", &legacy_utxos(), 1, 0, 0).is_err());
}

#[test]
fn zcash_fee_is_zip317() {
    let plan: serde_json::Value = serde_json::from_str(&legacy_plan("zcash", ZEC_FROM, ZEC_FROM, "700000000", &legacy_utxos(), 1, 0x7719_0ad9, 3_900_000).unwrap()).unwrap();
    assert_eq!(plan["fee"], 10_000);
}

/// Signs every hash in a plan's `sighashes` with `key`, as the MPC signer answers.
fn btc_mpc_sigs_any(plan: &str, key: [u8; 32]) -> String {
    use k256::ecdsa::SigningKey;
    let key = SigningKey::from_slice(&key).unwrap();
    let p: serde_json::Value = serde_json::from_str(plan).unwrap();
    let sigs: Vec<_> = p["sighashes"].as_array().unwrap().iter().rev().map(|h| {
        let (sig, recid) = key.sign_prehash_recoverable(&hex::decode(h.as_str().unwrap()).unwrap());
        serde_json::json!({ "big_r": { "affine_point": format!("{:02x}{}", 2 + recid.is_y_odd() as u8, hex::encode(sig.r().to_bytes())) }, "s": { "scalar": hex::encode(sig.s().to_bytes()) }, "recovery_id": recid.to_byte() })
    }).collect();
    serde_json::to_string(&sigs).unwrap()
}

fn ada_utxos() -> String {
    serde_json::json!([
        { "tx_hash": "cc".repeat(32), "tx_index": 0, "value": "10000000000", "asset_list": [] },
        { "tx_hash": "dd".repeat(32), "tx_index": 2, "value": "99000000000", "asset_list": [{ "policy_id": "ee" }] },
    ]).to_string()
}

#[test]
fn cardano_plan_signs_and_refuses() {
    use ed25519_dalek::{Signer, SigningKey};
    let key = SigningKey::from_bytes(&[0x22; 32]);
    let pk = format!("ed25519:{}", bs58::encode(key.verifying_key().to_bytes()).into_string());
    assert_eq!(chain_address("cardano", &pk).unwrap(), ADA_FROM);
    let to = "addr_test1qz2fxv2umyhttkxyxp8x0dlpdt3k6cwng5pxj3jhsydzer3jcu5d8ps7zex2k2xt3uqxgjqnnj83ws8lhrn648jjxtwq2ytjqp";
    let plan = cardano_plan(ADA_FROM, to, "5000000", &ada_utxos(), 140_000_000).unwrap();
    let p: serde_json::Value = serde_json::from_str(&plan).unwrap();
    assert_eq!(p["inputs"].as_array().unwrap().len(), 1, "the UTxO carrying tokens is skipped");
    let fee = p["fee"].as_u64().unwrap();
    assert!((165_000..200_000).contains(&fee), "{fee}");
    let d: serde_json::Value = serde_json::from_str(&describe_cardano_plan(&plan).unwrap()).unwrap();
    assert_eq!(d["pays"][0]["lovelace"], "5000000");
    let hash = hex::decode(p["hash"].as_str().unwrap()).unwrap();
    let sig = serde_json::json!({ "scheme": "Ed25519", "signature": key.sign(&hash).to_bytes().to_vec() }).to_string();
    let out: serde_json::Value = serde_json::from_str(&cardano_signed_tx(&plan, &sig, &pk).unwrap()).unwrap();
    // The fee was computed for exactly this size.
    let size = out["cbor"].as_str().unwrap().len() as u64 / 2;
    assert!(fee >= 44 * (size + 6) + 155_381 && fee <= 44 * (size + 16) + 155_381, "fee {fee} for {size} bytes");
    let other = SigningKey::from_bytes(&[0x23; 32]);
    let bad = serde_json::json!({ "signature": other.sign(&hash).to_bytes().to_vec() }).to_string();
    assert!(cardano_signed_tx(&plan, &bad, &pk).is_err());
    assert!(describe_cardano_plan(&plan.replace("5000000", "5000001")).is_err());
    assert!(cardano_plan(ADA_FROM, "addr1vx2fxv2umyhttkxyxp8x0dlpdt3k6cwng5pxj3jhsydzers66hrl8", "5000000", &ada_utxos(), 1).is_err(), "mainnet");
    assert!(cardano_plan(ADA_FROM, to, "500000", &ada_utxos(), 1).is_err(), "below the minimum ADA");
}

/// Fixed vectors, each from the chain's own reference library on the same
/// plan (key 0x11…11, inputs aa…:1 6 coins and bb…:0 2.5 coins, 7 coins to
/// self): Zcash from zcash_primitives 0.31 (txid and both ZIP-244 hashes;
/// the raw transaction parses and round-trips), Bitcoin Cash from
/// bitcore-lib-cash 10 (identical raw transaction), Dogecoin from
/// bitcoinjs-lib 6 (`hashForSignature`, both inputs).
#[test]
fn legacy_matches_reference_libraries() {
    for (chain, from, rate, txid, hashes) in [
        ("zcash", ZEC_FROM, 1, "6b3382e5bf2d4c400dfdf1a6c7ece12bc631de34e8e0d87356ed0f6530018242", ["d94586719b3fdf4a278657190ffec949bd76d1546529d9d462992639539f4e52", "97ac0fd59bc4416181ec6af4e039ce5f2b2c56bae56c91a0441f5188951e4c31"]),
        ("bitcoin-cash", BCH_FROM, 1, "83531402da639da222b22dc2b595870388d0ef58915a3c1172d17258415481a8", ["", ""]),
        ("dogecoin", DOGE_FROM, 1000, "1c69d65937fc63127bc915283265cb457fffcabd5fab2c786f79274ae6857223", ["3a187f024f749dda5fc65ad5aa65fa344e20474e4a253c35fd9f3240e7a0720e", "f96b9011bc8b570f12657887a992baa0d1ee078d92c6e69429fabeed0bd69caf"]),
    ] {
        let plan = legacy_plan(chain, from, from, "700000000", &legacy_utxos(), rate, 0x7719_0ad9, 3_900_000).unwrap();
        let p: serde_json::Value = serde_json::from_str(&plan).unwrap();
        for (i, h) in hashes.iter().enumerate() {
            if !h.is_empty() {
                assert_eq!(p["sighashes"][i], *h, "{chain} input {i}");
            }
        }
        let out: serde_json::Value = serde_json::from_str(&legacy_signed_tx(&plan, &btc_mpc_sigs_any(&plan, [0x11; 32])).unwrap()).unwrap();
        assert_eq!(out["txid"], txid, "{chain}");
    }
}

#[test]
fn extension_move_adds_then_removes() {
    let w = "0s3465557e026f471c0e84898f799c8032f356c77a";
    let (new, old) = ("cd1cd8fe1383641c975a832c077b612ef8c95935377636241d84803223f20deb", "2dc4e98d9de62211b1263a9a75d3c71caf45795719b790f12921198af309ba18");
    let m = extension_move_message("testnet", w, 7, 1_800_000_000, new, &format!("[\"{old}\"]")).unwrap();
    let d: serde_json::Value = serde_json::from_str(&describe_message(&m).unwrap()).unwrap();
    assert_eq!(d["internal"][0]["op"], "add_extension");
    assert_eq!(d["internal"][0]["account_id"], new);
    assert_eq!(d["internal"][1]["op"], "remove_extension");
    assert_eq!(d["internal"][1]["account_id"], old);
    assert!(d["external"].as_array().unwrap().is_empty());
    assert!(extension_move_message("testnet", w, 7, 1_800_000_000, new, &format!("[\"{new}\"]")).is_err());
}
