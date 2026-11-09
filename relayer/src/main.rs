//! Marvii NEAR relayer (testnet). It pays gas and nothing else: every
//! request it forwards is checked again on-chain, by the wallet (the
//! passkey's signature) or the extension (the session rules). It only
//! relays for wallets built from our wallet code with our extension, so it
//! can't be used as a free relayer for other contracts.
//!
//! Environment:
//! - Hosted: `MARVII_RELAYER_SECRET_KEY` (`ed25519:…`, the paying account's
//!   key, the only secret), `MARVII_RELAYER_ACCOUNT`, `MARVII_NEAR_EXTENSION`,
//!   `MARVII_NEAR_KNOWN_EXTENSIONS` (comma-separated older versions) and
//!   `MARVII_NEAR_WALLET_CODE_HASH` (hex); `BIND=0.0.0.0`.
//! - Local: `MARVII_NEAR_KEYS`, the directory the devtool writes
//!   (`deployer.json`, `extension*.json`, `wallet_code_hash.txt`).
//! - Keys are never printed.
//! - `MARVII_NEAR_RPC` (default `https://rpc.testnet.fastnear.com`; NEAR's own
//!   `rpc.testnet.near.org` is deprecated and refuses requests with HTTP 429),
//!   `MARVII_NEAR_CHAIN` (default `testnet`), `MARVII_NEAR_SIGNER` (default
//!   `v1.signer-prod.testnet`), `PORT` (default 8787),
//!   `MARVII_PAGE_ORIGIN` (default `http://localhost:5173`).
//! - `MARVII_NAMES_REGISTRY` (e.g. `marvii.testnet`; locally, `names.json`'s
//!   account): the name registry. The relayer forwards NEAR that lands on a
//!   name to its wallet, paying the gas. It holds no key for the registry.

use anyhow::{Context, Result, anyhow};
use axum::{Json, Router, extract::{Path, Query, State}, http::{HeaderValue, StatusCode}, routing::{get, post}};
use defuse_wallet::RequestMessage;
use defuse_wallet_sdk::relayer::{WalletRelayRequest, WalletRelayer};
use ed25519_dalek::SigningKey;
use near_kit::{
    Gas, Near, NearToken,
    rpc::{BlockReference, ExecutionStatus, FinalExecutionOutcome, Finality, RpcError},
    transaction::Final,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashSet, fs, path::PathBuf, sync::{Arc, Mutex}};
use tower_http::cors::CorsLayer;

fn env_or(k: &str, d: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| d.to_string())
}

/// What a new wallet gets once, towards grant deposits and storage.
const FUND: NearToken = NearToken::from_millinear(100);

struct App {
    near: Near,
    chain: String,
    rpc: String,
    signer: String,
    extension: String,
    code_hash: String,
    /// Every extension we've deployed (`extension*.json`). A wallet's
    /// address comes from the one it was created with; it may have moved to
    /// a newer one since.
    known_extensions: Vec<String>,
    funded: Mutex<HashSet<String>>,
    /// Agent requests the extension refused for being over the cap, waiting
    /// for the owner's passkey. In memory: a demo, not a queue.
    pending: Mutex<Vec<Value>>,
    /// The name registry, if names are enabled.
    names: Option<String>,
}

/// Below this, NEAR waiting on a name isn't worth a forward's gas yet.
const MIN_FORWARD: NearToken = NearToken::from_millinear(1);

type Shared = Arc<App>;
type Reply = Result<Json<Value>, (StatusCode, Json<Value>)>;

fn bad(e: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    // Logged so failures can be diagnosed from the host's logs. These are
    // validation and RPC messages; no key ever appears in them.
    let e = e.to_string();
    eprintln!("refused: {e}");
    (StatusCode::BAD_REQUEST, Json(json!({ "error": e })))
}

fn load_key(dir: &PathBuf, name: &str) -> Result<(String, String)> {
    let v: Value = serde_json::from_slice(&fs::read(dir.join(format!("{name}.json"))).with_context(|| format!("{name}.json"))?)?;
    let seed: [u8; 32] = hex::decode(v["seed_hex"].as_str().ok_or(anyhow!("seed_hex"))?)?
        .try_into()
        .map_err(|_| anyhow!("seed"))?;
    let mut b = seed.to_vec();
    b.extend(SigningKey::from_bytes(&seed).verifying_key().to_bytes());
    let account = v["account_id"].as_str().ok_or(anyhow!("account_id"))?.to_string();
    Ok((account, format!("ed25519:{}", bs58::encode(b).into_string())))
}

#[tokio::main]
async fn main() -> Result<()> {
    let rpc = env_or("MARVII_NEAR_RPC", "https://rpc.testnet.fastnear.com");
    let chain = env_or("MARVII_NEAR_CHAIN", "testnet");
    let mut names = std::env::var("MARVII_NAMES_REGISTRY").ok().filter(|s| !s.is_empty());
    let (payer, secret, extension, known_extensions, code_hash) = match std::env::var("MARVII_RELAYER_SECRET_KEY") {
        // Hosted: one secret (the paying account's key); the rest is public.
        Ok(secret) => {
            let ext = std::env::var("MARVII_NEAR_EXTENSION").context("set MARVII_NEAR_EXTENSION")?;
            let mut known: Vec<String> = env_or("MARVII_NEAR_KNOWN_EXTENSIONS", "").split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
            if !known.contains(&ext) {
                known.push(ext.clone());
            }
            (
                std::env::var("MARVII_RELAYER_ACCOUNT").context("set MARVII_RELAYER_ACCOUNT")?,
                secret,
                ext,
                known,
                std::env::var("MARVII_NEAR_WALLET_CODE_HASH").context("set MARVII_NEAR_WALLET_CODE_HASH")?,
            )
        }
        // Local: the keys directory the devtool writes.
        Err(_) => {
            let dir = PathBuf::from(std::env::var("MARVII_NEAR_KEYS").context("set MARVII_NEAR_KEYS or MARVII_RELAYER_SECRET_KEY")?);
            let (payer, secret) = load_key(&dir, "deployer")?;
            let (extension, _) = load_key(&dir, "extension")?;
            let mut known = vec![];
            for e in fs::read_dir(&dir)? {
                let name = e?.file_name().to_string_lossy().to_string();
                if let Some(stem) = name.strip_prefix("extension").and_then(|n| n.strip_suffix(".json")) {
                    if !stem.contains("broken") {
                        known.push(load_key(&dir, &format!("extension{stem}"))?.0);
                    }
                }
            }
            let code_hash = fs::read_to_string(dir.join("wallet_code_hash.txt"))?.trim().to_string();
            if names.is_none() {
                names = load_key(&dir, "names").ok().map(|(account, _)| account);
            }
            (payer, secret, extension, known, code_hash)
        }
    };
    let near = Near::custom(rpc.clone(), chain.clone()).credentials(secret, payer.as_str())?.build();
    let app = Arc::new(App {
        near,
        chain,
        rpc,
        signer: env_or("MARVII_NEAR_SIGNER", "v1.signer-prod.testnet"),
        extension,
        code_hash,
        known_extensions,
        funded: Mutex::new(HashSet::new()),
        pending: Mutex::new(Vec::new()),
        names,
    });
    if app.names.is_some() {
        tokio::spawn(forward_loop(app.clone()));
    }
    let origin = env_or("MARVII_PAGE_ORIGIN", "http://localhost:5173");
    let cors = CorsLayer::new()
        .allow_origin(origin.parse::<HeaderValue>()?)
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
        .allow_headers([axum::http::header::CONTENT_TYPE]);
    let router = Router::new()
        .route("/config", get(config))
        .route("/wallet/relay", post(relay))
        .route("/wallet/fund", post(fund))
        .route("/agent/execute", post(agent_execute))
        .route("/agent/propose", post(agent_propose))
        .route("/queued/{id}/execute", post(queued_execute))
        .route("/balance/cardano/{address}", get(cardano_balance))
        .route("/utxo/{chain}/address/{address}", get(utxo_address))
        .route("/utxo/{chain}/address/{address}/utxo", get(utxo_coins))
        .route("/utxo/{chain}/fees", get(utxo_fees))
        .route("/utxo/{chain}/tx", post(utxo_broadcast))
        .route("/names/forward", post(names_forward))
        .route("/pending", get(pending_list))
        .route("/pending/{id}", get(pending_get))
        .route("/pending/{id}/reject", post(pending_reject))
        .layer(cors)
        .with_state(app);
    let port = env_or("PORT", "8787");
    // Hosting platforms need 0.0.0.0; locally it stays on loopback.
    let host = env_or("BIND", "127.0.0.1");
    let listener = tokio::net::TcpListener::bind(format!("{host}:{port}")).await?;
    println!("relayer on 127.0.0.1:{port}, paying from {payer}");
    axum::serve(listener, router).await?;
    Ok(())
}

async fn config(State(a): State<Shared>) -> Json<Value> {
    Json(json!({
        "chain_id": a.chain, "rpc": a.rpc, "signer": a.signer,
        "extension": a.extension, "wallet_code_hash": a.code_hash, "names": a.names,
    }))
}

/// Forwards the NEAR waiting on a name to its wallet, if enough is there.
/// Only names in our registry; the forwarder sends to its fixed wallet, so
/// this can only ever move a deposit to where it was going.
async fn forward_name(a: &App, name: &str) -> Result<Option<String>, String> {
    let registry = a.names.as_deref().ok_or("names are not enabled")?;
    if !name.strip_suffix(registry).is_some_and(|l| l.ends_with('.') && !l[..l.len() - 1].contains('.')) {
        return Err("not one of our names".into());
    }
    let wallet: Option<String> = a.near.view(registry, "wallet_of").args(json!({ "name": name })).await.map_err(|e| e.to_string())?;
    if wallet.is_none() {
        return Err("not one of our names".into());
    }
    let waiting: String = a.near.view(name, "pending").await.map_err(|e| e.to_string())?;
    if waiting.parse::<u128>().map_err(|e| e.to_string())? < MIN_FORWARD.as_yoctonear() {
        return Ok(None);
    }
    let out = a.near.call(name, "forward").gas(Gas::from_tgas(10)).send().wait_until::<Final>().await.map_err(|e| e.to_string())?;
    Ok(Some(out.transaction_hash().to_string()))
}

#[derive(Deserialize)]
struct NameBody {
    name: String,
}

/// The page asks for its own name's deposits to be forwarded now.
async fn names_forward(State(a): State<Shared>, Json(b): Json<NameBody>) -> Reply {
    let tx = forward_name(&a, &b.name).await.map_err(bad)?;
    Ok(Json(json!({ "forwarded": tx.is_some(), "tx_hash": tx })))
}

/// Every 20 seconds, forwards deposits waiting on any name.
async fn forward_loop(a: Shared) {
    let registry = a.names.clone().unwrap();
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        let mut from = 0u32;
        loop {
            let page: Result<Vec<String>, _> = a.near.view(registry.as_str(), "names").args(json!({ "from_index": from, "limit": 100 })).await;
            let Ok(page) = page else { break };
            for name in &page {
                if let Ok(Some(tx)) = forward_name(&a, name).await {
                    println!("forwarded deposits on {name}, tx {tx}");
                }
            }
            if page.len() < 100 {
                break;
            }
            from += 100;
        }
    }
}

/// Checks the public key gives this wallet with our code and extension.
fn our_wallet(a: &App, public_key: &str, wallet: &str) -> Result<(), String> {
    for ext in &a.known_extensions {
        if marvii_near_core::wallet_account_id(&a.code_hash, public_key, ext)? == wallet {
            return Ok(());
        }
    }
    Err("not a Marvii wallet".into())
}

async fn initialized(a: &App, wallet: &str) -> Result<bool, String> {
    let id: near_kit::AccountId = wallet.parse().map_err(|_| "bad account id".to_string())?;
    match a.near.rpc().view_account(&id, BlockReference::Finality(Finality::Final)).await {
        // has_contract() only looks at code_hash; our wallets run global code.
        Ok(acc) => Ok(acc.has_contract() || acc.global_contract_hash.is_some() || acc.global_contract_account_id.is_some()),
        Err(RpcError::AccountNotFound { .. }) => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

/// What the page needs to show the result: success, failures, logs, and
/// any value the MPC signer returned.
fn summarize(a: &App, out: &FinalExecutionOutcome) -> Value {
    let mut failures = vec![];
    let mut logs = vec![];
    let mut mpc_signature = Value::Null;
    // A request can ask for several signatures (one per Bitcoin input).
    let mut mpc_signatures = vec![];
    for o in &out.receipts_outcome {
        logs.extend(o.outcome.logs.iter().cloned());
        match &o.outcome.status {
            ExecutionStatus::Failure(e) => failures.push(json!({ "on": o.outcome.executor_id.to_string(), "error": format!("{e:?}") })),
            ExecutionStatus::SuccessValue(v) if o.outcome.executor_id.as_str() == a.signer && !v.is_empty() => {
                mpc_signature = serde_json::from_slice(&v[..]).unwrap_or(Value::Null);
                mpc_signatures.push(mpc_signature.clone());
            }
            _ => {}
        }
    }
    json!({
        "tx_hash": out.transaction_hash().to_string(),
        "ok": failures.is_empty() && out.is_success(),
        "failures": failures,
        "logs": logs,
        "mpc_signature": mpc_signature,
        "mpc_signatures": mpc_signatures,
    })
}

#[derive(Deserialize)]
struct RelayBody {
    /// The passkey's P-256 public key, SEC1 hex.
    public_key: String,
    msg: RequestMessage,
    proof: String,
    /// Set when this request is the owner approving a pending agent request.
    #[serde(default)]
    pending_id: Option<u64>,
}

/// Relays a passkey-signed request to the wallet's `w_execute_signed`,
/// deploying the wallet first if needed.
async fn relay(State(a): State<Shared>, Json(b): Json<RelayBody>) -> Reply {
    our_wallet(&a, &b.public_key, b.msg.signer_id.as_str()).map_err(bad)?;
    if b.msg.chain_id != a.chain {
        return Err(bad("wrong chain_id"));
    }
    // Check the proof here too, so a bad one doesn't cost gas.
    let msg_json = serde_json::to_string(&b.msg).map_err(bad)?;
    if !marvii_near_core::verify_proof(&b.public_key, &msg_json, &b.proof).map_err(bad)? {
        return Err(bad("passkey signature does not verify"));
    }
    let mut req = WalletRelayRequest::new(b.msg.clone(), b.proof);
    if !initialized(&a, b.msg.signer_id.as_str()).await.map_err(bad)? {
        // The state the wallet was created with: the extension version whose
        // state init gives this exact address.
        let mut found = None;
        for ext in &a.known_extensions {
            let si = marvii_near_core::state_init(&a.code_hash, &b.public_key, ext).map_err(bad)?;
            if si.derive_account_id().as_str() == b.msg.signer_id.as_str() {
                found = Some(si);
                break;
            }
        }
        req = req.deterministic_state_init(found.ok_or_else(|| bad("not a Marvii wallet"))?);
    }
    let sent = a.near.relay_wallet_msg(req).await.map_err(bad)?;
    let out = sent.status(&a.near).wait_until::<Final>().await.map_err(bad)?;
    let s = summarize(&a, &out);
    // The owner approved a pending request: hand the MPC signature to the
    // agent, which checks it against its own transaction before broadcasting.
    if let (Some(id), false) = (b.pending_id, s["mpc_signature"].is_null()) {
        let mut p = a.pending.lock().unwrap();
        if let Some(item) = p.iter_mut().find(|i| i["id"] == id && i["wallet"] == b.msg.signer_id.as_str()) {
            item["status"] = "approved".into();
            item["mpc_signature"] = s["mpc_signature"].clone();
            item["near_tx"] = s["tx_hash"].clone();
        }
    }
    Ok(Json(s))
}

#[derive(Deserialize)]
struct FundBody {
    public_key: String,
    wallet: String,
}

/// Testnet only: gives a new Marvii wallet 0.1 NEAR once, so it can pay
/// the extension's grant deposit.
async fn fund(State(a): State<Shared>, Json(b): Json<FundBody>) -> Reply {
    our_wallet(&a, &b.public_key, &b.wallet).map_err(bad)?;
    if !a.funded.lock().unwrap().insert(b.wallet.clone()) {
        return Err(bad("already funded"));
    }
    let out = a.near.transfer(b.wallet.as_str(), FUND).wait_until::<Final>().await.map_err(bad)?;
    Ok(Json(summarize(&a, &out)))
}

#[derive(Deserialize)]
struct AgentBody {
    msg: String,
    signature: String,
}

/// Forwards an agent's signed session message to the extension. The
/// extension checks the signature and every rule; this only pays the gas.
async fn agent_execute(State(a): State<Shared>, Json(b): Json<AgentBody>) -> Reply {
    let m: Value = serde_json::from_str(&b.msg).map_err(bad)?;
    if m["extension"].as_str() != Some(a.extension.as_str()) {
        return Err(bad("message is for another extension"));
    }
    let out = a
        .near
        .call(a.extension.as_str(), "execute")
        .args(json!({ "msg": b.msg, "signature": b.signature }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(300))
        .send()
        .wait_until::<Final>()
        .await
        .map_err(bad)?;
    let mut s = summarize(&a, &out);
    let reason = refusal(&s);
    // Over the cap: keep it for the owner to approve with the passkey.
    if !s["ok"].as_bool().unwrap_or(false) && m["action"]["type"] == "evm_tx" && reason.as_deref() == Some("over the daily cap") {
        let mut p = a.pending.lock().unwrap();
        let id = p.len() as u64 + 1;
        p.push(json!({
            "id": id, "wallet": m["wallet"], "key_id": m["key_id"], "tx": m["action"]["tx"],
            "reason": reason, "status": "pending", "mpc_signature": null,
        }));
        s["pending_id"] = id.into();
    }
    s["reason"] = reason.map(Value::String).unwrap_or(Value::Null);
    // For an EVM transaction, also return the hash that was signed, so the
    // agent can attach the signature and broadcast.
    if m["action"]["type"] == "evm_tx" {
        let h: Result<String, _> = a.near.view(a.extension.as_str(), "evm_signing_hash").args(json!({ "tx": m["action"]["tx"] })).await;
        s["evm_signing_hash"] = h.map(Value::String).unwrap_or(Value::Null);
    }
    Ok(Json(s))
}

/// The extension's panic message, if a receipt failed with one.
fn refusal(s: &Value) -> Option<String> {
    let text = s["failures"].to_string();
    let i = text.find("panicked: ")? + "panicked: ".len();
    let rest = &text[i..];
    let end = rest.find(['\\', '"']).unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

#[derive(Deserialize)]
struct WalletQuery {
    wallet: String,
}

async fn pending_list(State(a): State<Shared>, Query(q): Query<WalletQuery>) -> Json<Value> {
    let p = a.pending.lock().unwrap();
    Json(Value::Array(p.iter().filter(|i| i["wallet"] == q.wallet.as_str()).cloned().collect()))
}

async fn pending_get(State(a): State<Shared>, Path(id): Path<u64>) -> Reply {
    let p = a.pending.lock().unwrap();
    p.iter().find(|i| i["id"] == id).cloned().map(Json).ok_or_else(|| bad("no such request"))
}

/// Testnet demo: no auth on reject. Rejecting only drops the request; it
/// can't move anything.
async fn pending_reject(State(a): State<Shared>, Path(id): Path<u64>) -> Reply {
    let mut p = a.pending.lock().unwrap();
    let item = p.iter_mut().find(|i| i["id"] == id && i["status"] == "pending").ok_or_else(|| bad("no such pending request"))?;
    item["status"] = "rejected".into();
    Ok(Json(item.clone()))
}

/// Forwards an agent's proposal (a new recipient or venue) to the extension,
/// which queues it behind the delay. Returns the queue id and ready time.
async fn agent_propose(State(a): State<Shared>, Json(b): Json<AgentBody>) -> Reply {
    let m: Value = serde_json::from_str(&b.msg).map_err(bad)?;
    if m["extension"].as_str() != Some(a.extension.as_str()) {
        return Err(bad("message is for another extension"));
    }
    let out = a
        .near
        .call(a.extension.as_str(), "propose")
        .args(json!({ "msg": b.msg, "signature": b.signature }))
        .gas(Gas::from_tgas(50))
        .send()
        .wait_until::<Final>()
        .await
        .map_err(bad)?;
    let mut s = summarize(&a, &out);
    s["reason"] = refusal(&s).map(Value::String).unwrap_or(Value::Null);
    // From the extension's log: `queued wallet=… key_id=… id=N ready_at_ms=M`.
    let line = s["logs"].as_array().and_then(|l| l.iter().filter_map(Value::as_str).find(|l| l.starts_with("queued ")).map(str::to_string));
    if let Some(l) = line {
        for part in l.split(' ') {
            if let Some(v) = part.strip_prefix("id=") {
                s["queue_id"] = v.parse::<u64>().map(Value::from).unwrap_or(Value::Null);
            }
            if let Some(v) = part.strip_prefix("ready_at_ms=") {
                s["ready_at_ms"] = v.parse::<u64>().map(Value::from).unwrap_or(Value::Null);
            }
        }
    }
    Ok(Json(s))
}

/// Runs a queued request after its delay. The extension checks the delay
/// and the session again; this only pays the gas.
async fn queued_execute(State(a): State<Shared>, Path(id): Path<u64>) -> Reply {
    let out = a
        .near
        .call(a.extension.as_str(), "execute_queued")
        .args(json!({ "id": id }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(300))
        .send()
        .wait_until::<Final>()
        .await
        .map_err(bad)?;
    let mut s = summarize(&a, &out);
    s["reason"] = refusal(&s).map(Value::String).unwrap_or(Value::Null);
    Ok(Json(s))
}

/// Cardano preprod balance in lovelace. Koios answers browsers' preflight
/// but leaves the CORS header off its POST responses, so the page asks here.
/// Only testnet addresses, and only this one read.
/// Bitcoin testnet4 and Litecoin testnet go through here first: some
/// browsers can't reach the explorers directly (blockers, rate limits). The
/// page falls back to calling them itself. Only these four read-and-send
/// paths, for well-formed addresses.
fn utxo_api(chain: &str) -> Result<&'static str, (StatusCode, Json<Value>)> {
    match chain {
        "bitcoin" => Ok("https://mempool.space/testnet4/api"),
        "litecoin" => Ok("https://litecoinspace.org/testnet/api"),
        _ => Err(bad("unknown chain")),
    }
}

fn utxo_address_ok(a: &str) -> Result<(), (StatusCode, Json<Value>)> {
    if a.is_empty() || a.len() > 100 || !a.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(bad("not an address"));
    }
    Ok(())
}

async fn utxo_get(url: String) -> Reply {
    let r = reqwest::Client::new().get(url).timeout(std::time::Duration::from_secs(10)).send().await.map_err(bad)?;
    if !r.status().is_success() {
        return Err(bad(format!("explorer returned {}", r.status())));
    }
    Ok(Json(r.json().await.map_err(bad)?))
}

async fn utxo_address(Path((chain, address)): Path<(String, String)>) -> Reply {
    utxo_address_ok(&address)?;
    utxo_get(format!("{}/address/{address}", utxo_api(&chain)?)).await
}

async fn utxo_coins(Path((chain, address)): Path<(String, String)>) -> Reply {
    utxo_address_ok(&address)?;
    utxo_get(format!("{}/address/{address}/utxo", utxo_api(&chain)?)).await
}

async fn utxo_fees(Path(chain): Path<String>) -> Reply {
    utxo_get(format!("{}/v1/fees/recommended", utxo_api(&chain)?)).await
}

/// Broadcasts a signed transaction (hex). The explorer checks it; the
/// relayer only passes it on.
async fn utxo_broadcast(Path(chain): Path<String>, body: String) -> Reply {
    let raw = body.trim();
    if raw.is_empty() || raw.len() > 200_000 || !raw.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(bad("not a raw transaction"));
    }
    let r = reqwest::Client::new()
        .post(format!("{}/tx", utxo_api(&chain)?))
        .body(raw.to_string())
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(bad)?;
    let ok = r.status().is_success();
    let text = r.text().await.map_err(bad)?.trim().to_string();
    if !ok {
        return Err(bad(text));
    }
    Ok(Json(json!({ "txid": text })))
}

async fn cardano_balance(Path(address): Path<String>) -> Reply {
    if !address.starts_with("addr_test1") || address.len() > 120 || !address.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(bad("not a Cardano testnet address"));
    }
    let r: Value = reqwest::Client::new()
        .post("https://preprod.koios.rest/api/v1/address_info")
        .json(&json!({ "_addresses": [address] }))
        .timeout(std::time::Duration::from_secs(8))
        .send()
        .await
        .map_err(bad)?
        .json()
        .await
        .map_err(bad)?;
    let lovelace = r.get(0).and_then(|a| a["balance"].as_str()).unwrap_or("0").to_string();
    Ok(Json(json!({ "lovelace": lovelace })))
}
