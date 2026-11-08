//! Testnet tool for the Marvii NEAR wallet (hackathon).
//!
//!   devtool publish-wallet <wallet.wasm>         deploy the wallet code as an immutable global contract
//!   devtool deploy-extension <extension.wasm>    create the extension's account, deploy and initialize it
//!   devtool e2e                                  passkey wallet + agent session + Chain Signatures request
//!
//! Keys live in $MARVII_NEAR_KEYS (a directory outside the repo) and are
//! never printed. Testnet only.

use anyhow::{Context, Result, anyhow, bail};
use defuse_crypto::p256::p256::ecdsa::SigningKey as P256Key;
use defuse_wallet::{NearPromise, NearToken, Gas, actions::FunctionCall, GlobalContractId};
use defuse_wallet_sdk::{Wallet, WalletBuilder, mpc::kdf::{DeriveExt, crypto::secp256k1::{Secp256k1, Secp256k1UncompressedPublicKey}}};
use defuse_wallet_webauthn::{WalletWebauthn, mock::MockWalletWebauthnSigner, p256::P256, webauthn::IgnoreUserVerification};
use ed25519_dalek::{Signer as _, SigningKey};
use near_kit::{Near, rpc::ExecutionStatus, transaction::Final, protocol::PublishMode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

/// Defaults are NEAR testnet; set MARVII_NEAR_RPC, MARVII_NEAR_CHAIN and
/// MARVII_NEAR_SIGNER to rehearse on a local sandbox first.
fn env_or(k: &str, d: &str) -> String { std::env::var(k).unwrap_or_else(|_| d.to_string()) }
fn rpc() -> String { env_or("MARVII_NEAR_RPC", "https://rpc.testnet.near.org") }
fn chain() -> String { env_or("MARVII_NEAR_CHAIN", "testnet") }
fn signer() -> String { env_or("MARVII_NEAR_SIGNER", "v1.signer-prod.testnet") }
/// On a local sandbox there is no MPC network: stop once the sign request
/// reaches the signer's account.
/// How long requests to new recipients or venues wait (default 10 minutes
/// for the testnet demo; 24–48 hours in real use).
fn queue_delay_ms() -> u64 { env_or("MARVII_QUEUE_DELAY_MS", "600000").parse().expect("MARVII_QUEUE_DELAY_MS") }
fn sandbox() -> bool { std::env::var("MARVII_NEAR_SANDBOX").is_ok() }
const CHAIN_ID: u64 = 84532; // Base Sepolia: only signed here, never broadcast

fn keys_dir() -> Result<PathBuf> {
    Ok(PathBuf::from(std::env::var("MARVII_NEAR_KEYS").context("set MARVII_NEAR_KEYS")?))
}

struct Key { account_id: String, seed: [u8; 32] }

impl Key {
    fn load(name: &str) -> Result<Key> {
        let v: Value = serde_json::from_slice(&fs::read(keys_dir()?.join(format!("{name}.json")))?)?;
        let seed = hex::decode(v["seed_hex"].as_str().unwrap())?.try_into().map_err(|_| anyhow!("seed"))?;
        Ok(Key { account_id: v["account_id"].as_str().unwrap().to_string(), seed })
    }
    fn create(name: &str) -> Result<Key> {
        let path = keys_dir()?.join(format!("{name}.json"));
        if path.exists() {
            return Key::load(name);
        }
        let seed: [u8; 32] = rand_bytes();
        let pk = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
        let account_id = hex::encode(pk);
        fs::write(&path, json!({ "account_id": account_id, "seed_hex": hex::encode(seed), "public_key_hex": hex::encode(pk) }).to_string())?;
        Ok(Key { account_id, seed })
    }
    fn secret(&self) -> String {
        let sk = SigningKey::from_bytes(&self.seed);
        let mut b = self.seed.to_vec();
        b.extend(sk.verifying_key().to_bytes());
        format!("ed25519:{}", bs58::encode(b).into_string())
    }
    fn near(&self) -> Result<Near> {
        Ok(Near::custom(rpc(), chain()).credentials(self.secret(), self.account_id.as_str())?.build())
    }
}

fn rand_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("random");
    b
}

fn code_hash(wasm: &[u8]) -> [u8; 32] {
    Sha256::digest(wasm).into()
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("publish-wallet") => publish_wallet(&args[2]).await,
        Some("deploy-extension") => deploy_extension(&args[2]).await,
        Some("e2e") => e2e().await,
        Some("demo-wallet") => demo_wallet(None, None, false).await,
        Some("demo-cancel") => demo(None, None, false, Some(args.get(2).context("queued id")?.parse()?)).await,
        Some("demo-upgrade") => demo_wallet(None, None, true).await,
        Some("demo-grant") => demo_wallet(Some(args.get(2).context("agent public key hex")?.clone()), None, false).await,
        Some("demo-approve") => demo_wallet(None, Some(args.get(2).context("pending request id")?.parse()?), false).await,
        _ => bail!("usage: devtool publish-wallet <wasm> | deploy-extension <wasm> | e2e | demo-wallet | demo-upgrade | demo-cancel <queued id> | demo-grant <agent pubkey hex> | demo-approve <pending id>"),
    }
}

async fn publish_wallet(path: &str) -> Result<()> {
    let wasm = fs::read(path)?;
    let hash = code_hash(&wasm);
    let deployer = Key::load("deployer")?;
    let out = deployer.near()?.publish(wasm, PublishMode::Immutable).wait_until::<Final>().await?;
    if !out.is_success() {
        bail!("publish failed: {:?}", out.failure_message());
    }
    fs::write(keys_dir()?.join("wallet_code_hash.txt"), hex::encode(hash))?;
    println!("wallet code published, hash {} (base58 {}), tx {}", hex::encode(hash), bs58::encode(hash).into_string(), out.transaction_hash());
    Ok(())
}

async fn deploy_extension(path: &str) -> Result<()> {
    let wasm = fs::read(path)?;
    let deployer = Key::load("deployer")?;
    let ext = Key::create("extension")?;
    let near = deployer.near()?;
    let out = near.transfer(ext.account_id.as_str(), NearToken::from_near(4)).wait_until::<Final>().await?;
    if !out.is_success() {
        bail!("funding failed: {:?}", out.failure_message());
    }
    let en = ext.near()?;
    let out = en
        .transaction(ext.account_id.as_str())
        .deploy(wasm)
        .call("new")
        .args(json!({ "owner": deployer.account_id, "signer": signer(), "queue_delay_ms": queue_delay_ms() }))
        .gas(Gas::from_tgas(30))
        .send()
        .wait_until::<Final>()
        .await?;
    if !out.is_success() {
        bail!("deploy failed: {:?}", out.failure_message());
    }
    println!("extension {} deployed, tx {}", ext.account_id, out.transaction_hash());
    Ok(())
}

/// A demo wallet whose passkey key is kept in the keys directory
/// (`demo-passkey.json`), so it can be funded on other chains and used again.
/// A software P-256 key in real WebAuthn format, like `e2e`; testnet only.
async fn demo_wallet(grant_to: Option<String>, approve: Option<u64>, upgrade: bool) -> Result<()> {
    demo(grant_to, approve, upgrade, None).await
}

async fn demo(grant_to: Option<String>, approve: Option<u64>, upgrade: bool, cancel: Option<u64>) -> Result<()> {
    let deployer = Key::load("deployer")?;
    let ext = Key::load("extension")?;
    let near = deployer.near()?;
    let hash: [u8; 32] = hex::decode(fs::read_to_string(keys_dir()?.join("wallet_code_hash.txt"))?.trim())?.try_into().unwrap();
    let path = keys_dir()?.join("demo-passkey.json");
    // The wallet's address comes from its first state, which names the
    // extension it was created with; later extensions are added by the
    // passkey (demo-upgrade), so that one is kept here.
    let (seed, created_ext): ([u8; 32], String) = if path.exists() {
        let v: Value = serde_json::from_slice(&fs::read(&path)?)?;
        let seed = hex::decode(v["seed_hex"].as_str().unwrap())?.try_into().map_err(|_| anyhow!("seed"))?;
        (seed, v["created_with_extension"].as_str().unwrap_or(&ext.account_id).to_string())
    } else {
        let s = rand_bytes::<32>();
        fs::write(&path, json!({ "seed_hex": hex::encode(s), "created_with_extension": ext.account_id }).to_string())?;
        (s, ext.account_id.clone())
    };
    let passkey = P256Key::from_bytes(&seed.into())?;
    let passkey_sec1 = hex::encode(passkey.verifying_key().to_sec1_point(false).as_bytes());
    let wallet: Wallet<WalletWebauthn<P256, IgnoreUserVerification>> = WalletBuilder::new()
        .extension(created_ext.parse::<defuse_wallet::AccountId>()?)
        .build(GlobalContractId::CodeHash(hash), MockWalletWebauthnSigner::new(passkey))
        .with_chain_id(chain())
        .with_mpc_contract_id(signer().parse::<defuse_wallet::AccountId>()?)
        .with_client(near.clone())
        .with_relayer(near.clone());
    let wallet_id = wallet.account_id().to_string();
    wallet.initialize().await.context("initialize wallet")?;
    if upgrade && created_ext != ext.account_id {
        // The passkey moves the wallet to the current extension: add it, and
        // remove the one it was created with. The address doesn't change.
        use defuse_wallet::{Request, WalletOp};
        let req = Request {
            internal: vec![
                WalletOp::AddExtension { account_id: ext.account_id.parse()? },
                WalletOp::RemoveExtension { account_id: created_ext.parse()? },
            ],
            external: vec![],
        };
        let out = wallet.sign_and_send(req).await?.status(&near).wait_until::<Final>().await?;
        println!("  passkey moved the wallet to extension {}: {} (tx {})", ext.account_id, out.is_success(), out.transaction_hash());
    }
    let bal = near.balance(wallet_id.as_str()).await?;
    if bal.total < NearToken::from_millinear(100) {
        let out = near.transfer(wallet_id.as_str(), NearToken::from_millinear(200)).wait_until::<Final>().await?;
        if !out.is_success() {
            bail!("funding wallet failed");
        }
    }
    println!("demo wallet {wallet_id}");
    if !sandbox() {
        let mpc = wallet.mpc_signer::<Secp256k1>(0).await?;
        let k = Secp256k1UncompressedPublicKey::from(mpc.derive_public_key("evm")).to_string();
        let addr = marvii_near_core::evm_address(&k).map_err(|e| anyhow!(e))?;
        println!("  EVM address (every EVM chain): {addr}");
    }
    if let Some(id) = cancel {
        let req = NearPromise::new(ext.account_id.parse::<defuse_wallet::AccountId>()?)
            .function_call(FunctionCall::name("cancel_queued").args_json(json!({ "id": id })).gas(Gas::from_tgas(20)));
        let out = wallet.sign_and_send(req).await?.status(&near).wait_until::<Final>().await?;
        let ok = out.receipts_outcome.iter().any(|o| o.outcome.logs.iter().any(|l| l.starts_with("queued_cancelled")));
        println!("  passkey cancelled queued request {id}: {ok} (tx {})", out.transaction_hash());
    }
    if let Some(id) = approve {
        // The owner approves a pending agent request, as the page does: the
        // message and its hash are built by near/core from the transaction,
        // and the relayer hands the MPC signature to the agent.
        let relayer = env_or("RELAYER", "http://localhost:8787");
        let http = reqwest::Client::new();
        let item: Value = http.get(format!("{relayer}/pending/{id}")).send().await?.json().await?;
        if item["status"] != "pending" || item["wallet"] != wallet_id.as_str() {
            bail!("not a pending request for this wallet: {item}");
        }
        let tx = item["tx"].to_string();
        println!("  approving: {}", marvii_near_core::describe_evm_tx(&tx).map_err(|e| anyhow!(e))?);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs() as i64 - 60;
        let m = marvii_near_core::owner_evm_sign_message(&chain(), &wallet_id, rand_bytes::<4>().iter().fold(0u32, |a, b| a << 8 | *b as u32), now, &signer(), &tx).map_err(|e| anyhow!(e))?;
        let msg: defuse_wallet::RequestMessage = serde_json::from_str(&m)?;
        let (msg, proof) = wallet.sign(msg.request).await?;
        let out: Value = http
            .post(format!("{relayer}/wallet/relay"))
            .json(&json!({ "public_key": passkey_sec1, "msg": msg, "proof": proof, "pending_id": id }))
            .send().await?.json().await?;
        println!("  passkey approved: ok={} (NEAR tx {})", out["ok"], out["tx_hash"]);
    }
    if let Some(agent_pk) = grant_to {
        // Aave V3 on Base Sepolia, from bgd-labs/aave-address-book
        // (AaveV3BaseSepolia.sol), checked on-chain: the pool's
        // getReservesList() includes this USDC.
        let pool = "0x8bab6d1b75f19e9ed9fce8b9bd338844ff79ae27";
        let usdc = "0xba50cd2a20f6da35d788639e581bca8d0b5d4d5f";
        let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis() as u64;
        let grant = json!({
            "key_id": "aave-agent",
            "grant": {
                "public_key": agent_pk,
                "label": "Aave agent",
                "expires_at_ms": now_ms + 7 * 86_400_000,
                "inactivity_ms": 3 * 86_400_000u64,
                "near_venues": [],
                // supply(address,uint256,address,uint16) = 0x617ba037 (cast sig)
                "evm_venues": [{ "chain_id": CHAIN_ID, "to": pool, "selector": "0x617ba037" }],
                "near_recipients": [],
                "evm_recipients": [],
                "caps": [{ "asset": format!("evm:{CHAIN_ID}:{usdc}"), "daily": "50000000" }],
                "queue_new_targets": true
            }
        });
        let req = NearPromise::new(ext.account_id.parse::<defuse_wallet::AccountId>()?).function_call(
            FunctionCall::name("grant_session").args_json(grant).attach_deposit(NearToken::from_millinear(10)).gas(Gas::from_tgas(20)),
        );
        let out = wallet.sign_and_send(req).await?.status(&near).wait_until::<Final>().await?;
        let granted = out.receipts_outcome.iter().any(|o| o.outcome.logs.iter().any(|l| l.starts_with("session_granted")));
        println!("  passkey granted aave-agent: {granted} (tx {})", out.transaction_hash());
        if !granted {
            bail!("grant failed: {:?}", out.failure_message());
        }
    }
    Ok(())
}

async fn e2e() -> Result<()> {
    let deployer = Key::load("deployer")?;
    let ext = Key::load("extension")?;
    let near = deployer.near()?;
    let hash: [u8; 32] = hex::decode(fs::read_to_string(keys_dir()?.join("wallet_code_hash.txt"))?.trim())?.try_into().unwrap();

    // 1. A passkey wallet with our extension enabled from the start. The
    //    passkey here is a software P-256 key producing real WebAuthn
    //    assertions (the SDK's mock authenticator); the page uses a real one.
    let passkey = P256Key::from_bytes(&rand_bytes::<32>().into())?;
    let wallet: Wallet<WalletWebauthn<P256, IgnoreUserVerification>> = WalletBuilder::new()
        .extension(ext.account_id.parse::<defuse_wallet::AccountId>()?)
        .build(GlobalContractId::CodeHash(hash), MockWalletWebauthnSigner::new(passkey))
        .with_chain_id(chain())
        .with_mpc_contract_id(signer().parse::<defuse_wallet::AccountId>()?)
        .with_client(near.clone())
        .with_relayer(near.clone());
    let wallet_id = wallet.account_id().to_string();
    println!("wallet {wallet_id}");
    wallet.initialize().await.context("initialize wallet")?;
    println!("  initialized (state init + first passkey-signed request)");

    let out = near.transfer(wallet_id.as_str(), NearToken::from_millinear(200)).wait_until::<Final>().await?;
    if !out.is_success() {
        bail!("funding wallet failed");
    }

    // 2. The passkey grants an agent session: approve USDC to the pool only,
    //    up to 50 USDC a day, for 7 days. Addresses are placeholders: this
    //    run only signs, it doesn't broadcast to Base Sepolia.
    let agent = SigningKey::from_bytes(&rand_bytes());
    let pool = "0x2222222222222222222222222222222222222222";
    let usdc = "0x4444444444444444444444444444444444444444";
    let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis() as u64;
    let grant = json!({
        "key_id": "agent-1",
        "grant": {
            "public_key": hex::encode(agent.verifying_key().to_bytes()),
            "label": "Trading agent",
            "expires_at_ms": now_ms + 7 * 86_400_000,
            "inactivity_ms": 3 * 86_400_000u64,
            "near_venues": [],
            "evm_venues": [{ "chain_id": CHAIN_ID, "to": pool, "selector": "0x617ba037" }],
            "near_recipients": [],
            "evm_recipients": [],
            "caps": [{ "asset": format!("evm:{CHAIN_ID}:{usdc}"), "daily": "50000000" }]
        }
    });
    let req = NearPromise::new(ext.account_id.parse::<defuse_wallet::AccountId>()?).function_call(
        FunctionCall::name("grant_session")
            .args_json(grant)
            .attach_deposit(NearToken::from_millinear(10))
            .gas(Gas::from_tgas(20)),
    );
    let out = wallet.sign_and_send(req).await?.status(&near).wait_until::<Final>().await?;
    let granted = out.receipts_outcome.iter().any(|o| o.outcome.logs.iter().any(|l| l.starts_with("session_granted")));
    println!("  passkey granted agent-1: {granted} (tx {})", out.transaction_hash());
    if !granted {
        bail!("grant failed: {:?}", out.failure_message());
    }

    // 3. The agent asks for approve(pool, 20 USDC) on Base Sepolia.
    let mut data = vec![0x09, 0x5e, 0xa7, 0xb3];
    data.extend([0u8; 12]);
    data.extend(hex::decode(&pool[2..])?);
    data.extend([0u8; 16]);
    data.extend(20_000_000u128.to_be_bytes());
    let tx = json!({
        "chain_id": CHAIN_ID, "nonce": 0, "max_priority_fee_per_gas": "1000000", "max_fee_per_gas": "2000000000",
        "gas_limit": 100000, "to": usdc, "value": "0", "data": format!("0x{}", hex::encode(&data))
    });
    let msg = json!({
        "domain": "marvii-session-v1", "extension": ext.account_id, "wallet": wallet_id, "key_id": "agent-1",
        "nonce": 1, "deadline_ms": now_ms + 300_000, "action": { "type": "evm_tx", "tx": tx }
    })
    .to_string();
    let sig = agent.sign(&Sha256::digest(msg.as_bytes()));
    let out = near
        .call(ext.account_id.as_str(), "execute")
        .args(json!({ "msg": msg, "signature": hex::encode(sig.to_bytes()) }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(300))
        .send()
        .wait_until::<Final>()
        .await?;
    println!("  agent request sent (tx {})", out.transaction_hash());
    for o in &out.receipts_outcome {
        if let ExecutionStatus::Failure(e) = &o.outcome.status {
            println!("  receipt failed on {}: {e:?}", o.outcome.executor_id);
        }
    }
    if sandbox() {
        let reached = out.receipts_outcome.iter().any(|o| o.outcome.logs.iter().any(|l| l.starts_with("session_used")));
        println!("  sandbox: extension accepted the agent request: {reached}; receipts:");
        for o in &out.receipts_outcome {
            println!("    {} -> {:?}", o.outcome.executor_id, o.outcome.status);
        }
        return Ok(());
    }
    let response = out
        .receipts_outcome
        .iter()
        .filter(|o| o.outcome.executor_id.as_str() == signer())
        .find_map(|o| match &o.outcome.status {
            ExecutionStatus::SuccessValue(v) if !v.is_empty() => serde_json::from_slice::<Value>(&v[..]).ok(),
            _ => None,
        })
        .ok_or_else(|| anyhow!("no signature from the MPC signer"))?;
    println!("  MPC signature: {response}");

    // 4. Check it: the signature must recover to the wallet's derived key
    //    for path `evm` (one address on every EVM chain), over the hash the
    //    extension built.
    let signing_hash: String = near.view(ext.account_id.as_str(), "evm_signing_hash").args(json!({ "tx": tx })).await?;
    let mpc = wallet.mpc_signer::<Secp256k1>(0).await?;
    let derived = mpc.derive_public_key("evm");
    let derived_hex = Secp256k1UncompressedPublicKey::from(derived).to_string();
    let recovered = recover(&response, &hex::decode(&signing_hash)?)?;
    println!("  derived key  {derived_hex}");
    println!("  recovered    {recovered}");
    // The SDK prints the key as `secp256k1:<base58 of x||y>`; the recovered
    // key is hex `04||x||y`.
    let derived_xy = hex::encode(bs58::decode(derived_hex.trim_start_matches("secp256k1:")).into_vec()?);
    let ok = derived_xy == recovered[2..];
    println!("  signature matches the wallet's derived EVM key: {ok}");
    if !ok {
        bail!("signature does not match");
    }
    Ok(())
}

/// Recovers the uncompressed public key from an MPC secp256k1 response.
fn recover(resp: &Value, hash: &[u8]) -> Result<String> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
    let big_r = resp["big_r"]["affine_point"].as_str().ok_or_else(|| anyhow!("big_r"))?;
    let s = resp["s"]["scalar"].as_str().ok_or_else(|| anyhow!("s"))?;
    let rid = resp["recovery_id"].as_u64().ok_or_else(|| anyhow!("recovery_id"))? as u8;
    let r = hex::decode(&big_r[2..])?;
    let mut rs = r.clone();
    rs.extend(hex::decode(s)?);
    let sig = Signature::from_slice(&rs)?;
    let vk = VerifyingKey::recover_from_prehash(hash, &sig, RecoveryId::from_byte(rid).ok_or_else(|| anyhow!("rid"))?)?;
    Ok(hex::encode(vk.to_encoded_point(false).as_bytes()))
}
