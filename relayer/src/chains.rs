//! Reads and broadcasts for Cardano (Koios), Zcash (lightwalletd, gRPC),
//! Bitcoin Cash (Electrum, chipnet) and Dogecoin (an Esplora API). One shape for the page:
//! `GET /chain/{chain}/utxos/{address}`, `GET /chain/{chain}/balance/{address}`,
//! `GET /chain/{chain}/info`, `POST /chain/{chain}/tx` (hex).
//!
//! Testnet only. The relayer holds no key for any of these; it only reads
//! and passes signed transactions on, and each node checks them.

use axum::{Json, extract::Path, http::StatusCode};
use serde_json::{Value, json};
use std::time::Duration;

type Reply = Result<Json<Value>, (StatusCode, Json<Value>)>;

fn bad(e: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    super::bad(e)
}

fn env_or(k: &str, d: &str) -> String {
    std::env::var(k).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| d.to_string())
}

const KOIOS: &str = "https://preprod.koios.rest/api/v1";

fn check_address(chain: &str, a: &str) -> Result<(), (StatusCode, Json<Value>)> {
    let ok = match chain {
        "cardano" => a.starts_with("addr_test1") && a.len() <= 120 && a.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
        "zcash" | "bitcoin-cash" | "dogecoin" => marvii_near_core::legacy_script_pubkey(chain, a).is_ok(),
        _ => return Err(bad("unknown chain")),
    };
    if ok { Ok(()) } else { Err(bad(format!("not a {chain} testnet address"))) }
}

// ---- Cardano (Koios, preprod) ----

async fn koios_post(path: &str, body: Value) -> Result<Value, (StatusCode, Json<Value>)> {
    let r = reqwest::Client::new().post(format!("{KOIOS}{path}")).json(&body).timeout(Duration::from_secs(10)).send().await.map_err(bad)?;
    if !r.status().is_success() {
        return Err(bad(format!("Koios returned {}", r.status())));
    }
    r.json().await.map_err(bad)
}

// ---- Zcash (lightwalletd over gRPC, testnet) ----

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
fn pb_bytes(out: &mut Vec<u8>, field: u32, b: &[u8]) {
    pb_varint(out, (field << 3 | 2) as u64);
    pb_varint(out, b.len() as u64);
    out.extend(b);
}
fn pb_uint(out: &mut Vec<u8>, field: u32, v: u64) {
    pb_varint(out, (field << 3) as u64);
    pb_varint(out, v);
}

/// A protobuf field: varint value or length-delimited bytes.
enum Pb<'a> {
    Int(u64),
    Bytes(&'a [u8]),
}

fn pb_fields(mut b: &[u8]) -> Result<Vec<(u32, Pb<'_>)>, String> {
    fn varint(b: &mut &[u8]) -> Result<u64, String> {
        let mut v = 0u64;
        for i in 0..10 {
            let (&x, rest) = b.split_first().ok_or("truncated")?;
            *b = rest;
            v |= ((x & 0x7f) as u64) << (7 * i);
            if x & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err("bad varint".into())
    }
    let mut out = vec![];
    while !b.is_empty() {
        let key = varint(&mut b)?;
        let (field, wire) = ((key >> 3) as u32, key & 7);
        match wire {
            0 => out.push((field, Pb::Int(varint(&mut b)?))),
            2 => {
                let n = varint(&mut b)? as usize;
                if n > b.len() {
                    return Err("truncated".into());
                }
                out.push((field, Pb::Bytes(&b[..n])));
                b = &b[n..];
            }
            1 => b = b.get(8..).ok_or("truncated")?,
            5 => b = b.get(4..).ok_or("truncated")?,
            _ => return Err("unsupported wire type".into()),
        }
    }
    Ok(out)
}

async fn grpc(method: &str, msg: &[u8]) -> Result<Vec<u8>, (StatusCode, Json<Value>)> {
    let host = env_or("MARVII_ZCASH_LIGHTWALLETD", "https://testnet.zec.rocks");
    let mut body = vec![0u8];
    body.extend((msg.len() as u32).to_be_bytes());
    body.extend(msg);
    let r = reqwest::Client::builder()
        .http2_prior_knowledge()
        .build()
        .map_err(bad)?
        .post(format!("{host}/cash.z.wallet.sdk.rpc.CompactTxStreamer/{method}"))
        .header("content-type", "application/grpc")
        .header("te", "trailers")
        .body(body)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(bad)?;
    if let Some(s) = r.headers().get("grpc-status").and_then(|v| v.to_str().ok()) {
        if s != "0" {
            let m = r.headers().get("grpc-message").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
            return Err(bad(format!("lightwalletd: {m} ({s})")));
        }
    }
    let b = r.bytes().await.map_err(bad)?;
    // Concatenated frames (a stream); we read every message.
    let mut out = vec![];
    let mut rest = &b[..];
    while rest.len() >= 5 {
        let n = u32::from_be_bytes(rest[1..5].try_into().unwrap()) as usize;
        let frame = rest.get(5..5 + n).ok_or_else(|| bad("truncated gRPC frame"))?;
        // Each frame is length-prefixed again, so callers can split.
        pb_varint(&mut out, n as u64);
        out.extend(frame);
        rest = &rest[5 + n..];
    }
    Ok(out)
}

fn frames(b: &[u8]) -> Vec<&[u8]> {
    let mut out = vec![];
    let mut r = b;
    while !r.is_empty() {
        let mut n = 0usize;
        let mut i = 0;
        while i < r.len() {
            n |= ((r[i] & 0x7f) as usize) << (7 * i);
            i += 1;
            if r[i - 1] & 0x80 == 0 {
                break;
            }
        }
        out.push(&r[i..i + n]);
        r = &r[i + n..];
    }
    out
}

async fn zcash_utxos(address: &str) -> Result<Vec<Value>, (StatusCode, Json<Value>)> {
    let mut arg = vec![];
    pb_bytes(&mut arg, 1, address.as_bytes());
    pb_uint(&mut arg, 3, 50);
    let resp = grpc("GetAddressUtxos", &arg).await?;
    let mut out = vec![];
    for frame in frames(&resp) {
        for (f, v) in pb_fields(frame).map_err(bad)? {
            let (1, Pb::Bytes(u)) = (f, v) else { continue };
            let (mut txid, mut index, mut value) = (vec![], 0u64, 0u64);
            for (f, v) in pb_fields(u).map_err(bad)? {
                match (f, v) {
                    (1, Pb::Bytes(t)) => txid = t.to_vec(),
                    (2, Pb::Int(i)) => index = i,
                    (4, Pb::Int(z)) => value = z,
                    _ => {}
                }
            }
            if txid.len() != 32 {
                continue;
            }
            // lightwalletd sends the txid in internal byte order.
            txid.reverse();
            out.push(json!({ "txid": hex::encode(txid), "vout": index, "value": value }));
        }
    }
    Ok(out)
}

async fn zcash_info() -> Result<Value, (StatusCode, Json<Value>)> {
    let info = grpc("GetLightdInfo", &[]).await?;
    let mut branch = None;
    for frame in frames(&info) {
        for (f, v) in pb_fields(frame).map_err(bad)? {
            if let (6, Pb::Bytes(b)) = (f, v) {
                branch = u32::from_str_radix(std::str::from_utf8(b).unwrap_or(""), 16).ok();
            }
        }
    }
    let latest = grpc("GetLatestBlock", &[]).await?;
    let mut height = None;
    for frame in frames(&latest) {
        for (f, v) in pb_fields(frame).map_err(bad)? {
            if let (1, Pb::Int(h)) = (f, v) {
                height = Some(h);
            }
        }
    }
    let (Some(branch), Some(height)) = (branch, height) else { return Err(bad("lightwalletd gave no branch ID or height")) };
    Ok(json!({ "branch_id": branch, "height": height }))
}

// ---- Bitcoin Cash (Electrum over TLS, chipnet) ----

async fn electrum(method: &str, params: Value) -> Result<Value, (StatusCode, Json<Value>)> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let server = env_or("MARVII_BCH_ELECTRUM", "chipnet.bch.ninja:50002");
    let host = server.split(':').next().unwrap_or("").to_string();
    let run = async {
        let tcp = tokio::net::TcpStream::connect(&server).await.map_err(|e| e.to_string())?;
        // Two crypto providers are linked (ring here, aws-lc-rs via reqwest),
        // so name one: `builder()` would panic choosing between them.
        let config = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_root_certificates(rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() })
            .with_no_client_auth();
        let name = rustls::pki_types::ServerName::try_from(host).map_err(|e| e.to_string())?;
        let tls = tokio_rustls::TlsConnector::from(std::sync::Arc::new(config)).connect(name, tcp).await.map_err(|e| e.to_string())?;
        let (r, mut w) = tokio::io::split(tls);
        let req = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string() + "\n";
        w.write_all(req.as_bytes()).await.map_err(|e| e.to_string())?;
        let mut line = String::new();
        BufReader::new(r).read_line(&mut line).await.map_err(|e| e.to_string())?;
        let v: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
            return Err(e.get("message").and_then(|m| m.as_str()).unwrap_or("refused").to_string());
        }
        Ok::<Value, String>(v["result"].clone())
    };
    tokio::time::timeout(Duration::from_secs(15), run).await.map_err(|_| bad("Electrum timed out"))?.map_err(|e| bad(format!("Electrum: {e}")))
}

fn scripthash(chain: &str, address: &str) -> Result<String, (StatusCode, Json<Value>)> {
    use sha2::{Digest, Sha256};
    let script = hex::decode(marvii_near_core::legacy_script_pubkey(chain, address).map_err(bad)?).map_err(bad)?;
    let mut h: Vec<u8> = Sha256::digest(&script).to_vec();
    h.reverse();
    Ok(hex::encode(h))
}

async fn bch_utxos(address: &str) -> Result<Vec<Value>, (StatusCode, Json<Value>)> {
    let list = electrum("blockchain.scripthash.listunspent", json!([scripthash("bitcoin-cash", address)?])).await?;
    Ok(list
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        // CashTokens outputs are left alone.
        .filter(|u| u.get("token_data").is_none_or(|t| t.is_null()))
        .map(|u| json!({ "txid": u["tx_hash"], "vout": u["tx_pos"], "value": u["value"] }))
        .collect())
}

// ---- Dogecoin (an Esplora API, testnet) ----

/// QED's public Dogecoin testnet Esplora by default (the only free one
/// found, 9 November 2026); any Esplora-compatible URL can replace it.
fn doge_api() -> String {
    env_or("MARVII_DOGE_ESPLORA", "https://doge-electrs-testnet-demo.qed.me")
}

async fn esplora_get(path: &str) -> Result<Value, (StatusCode, Json<Value>)> {
    let r = reqwest::Client::new().get(format!("{}{path}", doge_api())).timeout(Duration::from_secs(10)).send().await.map_err(bad)?;
    if !r.status().is_success() {
        return Err(bad(format!("Dogecoin explorer returned {}", r.status())));
    }
    r.json().await.map_err(bad)
}

// ---- Routes ----

pub async fn utxos(Path((chain, address)): Path<(String, String)>) -> Reply {
    check_address(&chain, &address)?;
    Ok(Json(match chain.as_str() {
        "cardano" => koios_post("/address_utxos", json!({ "_addresses": [address] })).await?,
        "zcash" => Value::Array(zcash_utxos(&address).await?),
        "dogecoin" => {
            let list = esplora_get(&format!("/address/{address}/utxo")).await?;
            Value::Array(list.as_array().cloned().unwrap_or_default().into_iter().map(|u| json!({ "txid": u["txid"], "vout": u["vout"], "value": u["value"] })).collect())
        }
        _ => Value::Array(bch_utxos(&address).await?),
    }))
}

pub async fn balance(Path((chain, address)): Path<(String, String)>) -> Reply {
    check_address(&chain, &address)?;
    let total: u64 = match chain.as_str() {
        "cardano" => {
            let r = koios_post("/address_info", json!({ "_addresses": [address] })).await?;
            r.get(0).and_then(|a| a["balance"].as_str()).unwrap_or("0").parse().unwrap_or(0)
        }
        "zcash" => zcash_utxos(&address).await?.iter().map(|u| u["value"].as_u64().unwrap_or(0)).sum(),
        "dogecoin" => {
            let j = esplora_get(&format!("/address/{address}")).await?;
            let n = |k: &str, f: &str| j[k][f].as_i64().unwrap_or(0);
            (n("chain_stats", "funded_txo_sum") - n("chain_stats", "spent_txo_sum") + n("mempool_stats", "funded_txo_sum") - n("mempool_stats", "spent_txo_sum")).max(0) as u64
        }
        _ => {
            let b = electrum("blockchain.scripthash.get_balance", json!([scripthash("bitcoin-cash", &address)?])).await?;
            (b["confirmed"].as_i64().unwrap_or(0) + b["unconfirmed"].as_i64().unwrap_or(0)).max(0) as u64
        }
    };
    Ok(Json(json!({ "balance": total.to_string() })))
}

pub async fn info(Path(chain): Path<String>) -> Reply {
    Ok(Json(match chain.as_str() {
        "cardano" => {
            let r = reqwest::Client::new().get(format!("{KOIOS}/tip")).timeout(Duration::from_secs(10)).send().await.map_err(bad)?;
            let t: Value = r.json().await.map_err(bad)?;
            json!({ "slot": t[0]["abs_slot"] })
        }
        "zcash" => zcash_info().await?,
        _ => return Err(bad("unknown chain")),
    }))
}

pub async fn broadcast(Path(chain): Path<String>, body: String) -> Reply {
    let raw = body.trim();
    if raw.is_empty() || raw.len() > 200_000 || !raw.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(bad("not a raw transaction"));
    }
    let bytes = hex::decode(raw).map_err(bad)?;
    match chain.as_str() {
        "cardano" => {
            // Koios balances requests across nodes; one with its node's
            // socket down answers TxCmdTxSubmitConnectionError. Try again.
            for attempt in 1..=3 {
                let r = reqwest::Client::new()
                    .post(format!("{KOIOS}/submittx"))
                    .header("content-type", "application/cbor")
                    .body(bytes.clone())
                    .timeout(Duration::from_secs(15))
                    .send()
                    .await
                    .map_err(bad)?;
                let ok = r.status().is_success();
                let text = r.text().await.map_err(bad)?;
                if ok {
                    return Ok(Json(json!({ "txid": text.trim().trim_matches('"') })));
                }
                if !text.contains("ConnectionError") || attempt == 3 {
                    return Err(bad(text));
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            unreachable!()
        }
        "zcash" => {
            let mut m = vec![];
            pb_bytes(&mut m, 1, &bytes);
            let resp = grpc("SendTransaction", &m).await?;
            let (mut code, mut message) = (0u64, String::new());
            for frame in frames(&resp) {
                for (f, v) in pb_fields(frame).map_err(bad)? {
                    match (f, v) {
                        (1, Pb::Int(c)) => code = c,
                        (2, Pb::Bytes(b)) => message = String::from_utf8_lossy(b).to_string(),
                        _ => {}
                    }
                }
            }
            if code != 0 {
                return Err(bad(format!("Zcash refused: {message}")));
            }
            Ok(Json(json!({ "txid": message.trim().trim_matches('"') })))
        }
        "bitcoin-cash" => Ok(Json(json!({ "txid": electrum("blockchain.transaction.broadcast", json!([raw])).await? }))),
        "dogecoin" => {
            let r = reqwest::Client::new().post(format!("{}/tx", doge_api())).body(raw.to_string()).timeout(Duration::from_secs(15)).send().await.map_err(bad)?;
            let ok = r.status().is_success();
            let text = r.text().await.map_err(bad)?.trim().to_string();
            if !ok {
                return Err(bad(text));
            }
            Ok(Json(json!({ "txid": text })))
        }
        _ => Err(bad("unknown chain")),
    }
}

// ---- NEAR Intents (1Click), quotes only ----
//
// NEAR Intents runs on mainnet only. The relayer asks 1Click for dry quotes
// (always `dry: true`, so no deposit address is ever created), between
// balances held inside NEAR Intents by the account itself.

const ONECLICK: &str = "https://1click.chaindefuser.com/v0";

/// The assets the page offers: each chain's native coin as 1Click lists it
/// (`/v0/tokens`, checked 9 November 2026), plus USDC on NEAR.
const INTENTS_ASSETS: &[&str] = &[
    "nep141:wrap.near",
    "nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1",
    "nep141:eth.omft.near",
    "nep141:btc.omft.near",
    "nep141:sol.omft.near",
    "nep141:zec.omft.near",
    "nep141:doge.omft.near",
    "nep141:xrp.omft.near",
    "nep141:tron.omft.near",
    "nep141:cardano.omft.near",
    "nep141:ltc.omft.near",
    "nep141:bch.omft.near",
];

pub async fn intents_tokens() -> Reply {
    let r = reqwest::Client::new().get(format!("{ONECLICK}/tokens")).timeout(Duration::from_secs(10)).send().await.map_err(bad)?;
    let all: Value = r.json().await.map_err(bad)?;
    let mut out: Vec<Value> = all
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|t| t["assetId"].as_str().is_some_and(|a| INTENTS_ASSETS.contains(&a)))
        .map(|t| json!({ "asset": t["assetId"], "symbol": t["symbol"], "chain": t["blockchain"], "decimals": t["decimals"], "price": t["price"] }))
        .collect();
    out.sort_by_key(|t| INTENTS_ASSETS.iter().position(|a| Some(*a) == t["asset"].as_str()));
    Ok(Json(Value::Array(out)))
}

#[derive(serde::Deserialize)]
pub struct QuoteBody {
    from: String,
    to: String,
    amount: String,
    account: String,
}

pub async fn intents_quote(Json(b): Json<QuoteBody>) -> Reply {
    if !INTENTS_ASSETS.contains(&b.from.as_str()) || !INTENTS_ASSETS.contains(&b.to.as_str()) || b.from == b.to {
        return Err(bad("pick two different assets"));
    }
    if b.amount.is_empty() || b.amount.len() > 40 || !b.amount.chars().all(|c| c.is_ascii_digit()) {
        return Err(bad("amount must be a whole number of base units"));
    }
    if b.account.len() < 2 || b.account.len() > 64 || !b.account.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-_.".contains(c)) {
        return Err(bad("not a NEAR account"));
    }
    let deadline = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(bad)?.as_secs() + 600;
    let req = json!({
        "dry": true,
        "swapType": "EXACT_INPUT",
        "slippageTolerance": 100,
        "originAsset": b.from,
        "depositType": "INTENTS",
        "destinationAsset": b.to,
        "amount": b.amount,
        "refundTo": b.account,
        "refundType": "INTENTS",
        "recipient": b.account,
        "recipientType": "INTENTS",
        "deadline": format_rfc3339(deadline),
    });
    let r = reqwest::Client::new().post(format!("{ONECLICK}/quote")).json(&req).timeout(Duration::from_secs(20)).send().await.map_err(bad)?;
    let ok = r.status().is_success();
    let v: Value = r.json().await.map_err(bad)?;
    if !ok {
        return Err(bad(v["message"].as_str().unwrap_or("no quote")));
    }
    Ok(Json(json!({ "quote": v["quote"], "timestamp": v["timestamp"] })))
}

// ---- NEAR Intents top-ups (real quotes) ----
//
// A top-up moves the account's own funds from one chain to its own address
// on another. The relayer only fetches 1Click's signed quote and rebuilds the
// exact JSON 1Click signed (one-click-sdk-typescript, quote-signature.ts);
// the extension checks the signature and every rule itself (intents.rs), so
// nothing here is trusted.

#[derive(serde::Deserialize)]
pub struct DepositQuoteBody {
    from: String,
    to: String,
    amount: String,
    /// The account's own address on the origin chain.
    refund_to: String,
    /// The account's own address on the destination chain.
    recipient: String,
    #[serde(default)]
    slippage_bps: Option<u64>,
}

fn plain(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && s.chars().all(|c| c.is_ascii_alphanumeric() || ":-_.".contains(c))
}

/// json-stable-stringify: keys sorted, no spaces, undefined (absent) dropped.
pub fn stable(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().filter(|k| !m[*k].is_null()).collect();
            keys.sort();
            let parts: Vec<String> = keys.iter().map(|k| format!("{}:{}", Value::String((*k).clone()), stable(&m[*k]))).collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(a) => format!("[{}]", a.iter().map(stable).collect::<Vec<_>>().join(",")),
        _ => v.to_string(),
    }
}

/// The object 1Click signs: chosen request fields, then the quote's fields
/// (which overwrite, as `deadline` does), then the timestamp.
pub fn signed_quote_json(r: &Value) -> String {
    let (q, o) = (&r["quoteRequest"], &r["quote"]);
    let truthy = |v: &Value| match v {
        Value::Null | Value::Bool(false) => Value::Null,
        Value::String(s) if s.is_empty() => Value::Null,
        Value::Number(n) if n.as_f64() == Some(0.0) => Value::Null,
        _ => v.clone(),
    };
    let mut m = serde_json::Map::new();
    for k in ["dry", "swapType", "slippageTolerance", "originAsset", "depositType", "destinationAsset", "amount", "refundTo", "refundType", "recipient", "recipientType", "deadline"] {
        m.insert(k.into(), q[k].clone());
    }
    for k in ["quoteWaitingTimeMs", "referral", "virtualChainRecipient", "virtualChainRefundRecipient", "customRecipientMsg"] {
        m.insert(k.into(), truthy(&q[k]));
    }
    for k in ["amountIn", "amountInFormatted", "amountInUsd", "minAmountIn", "amountOut", "amountOutFormatted", "amountOutUsd", "minAmountOut"] {
        m.insert(k.into(), o[k].clone());
    }
    if q["dry"] != Value::Bool(true) {
        for k in ["depositAddress", "depositMemo", "deadline", "timeWhenInactive", "timeEstimate", "refundFee", "withdrawFee"] {
            m.insert(k.into(), truthy(&o[k]));
        }
    }
    m.insert("timestamp".into(), r["timestamp"].clone());
    stable(&Value::Object(m))
}

pub async fn intents_deposit_quote(Json(b): Json<DepositQuoteBody>) -> Reply {
    if !plain(&b.from, 120) || !plain(&b.to, 120) || b.from == b.to {
        return Err(bad("pick two different assets"));
    }
    if b.amount.is_empty() || b.amount.len() > 40 || !b.amount.chars().all(|c| c.is_ascii_digit()) {
        return Err(bad("amount must be a whole number of base units"));
    }
    if !plain(&b.refund_to, 100) || !plain(&b.recipient, 100) {
        return Err(bad("bad address"));
    }
    let deadline = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(bad)?.as_secs() + 600;
    let req = json!({
        "dry": false,
        "swapType": "EXACT_INPUT",
        "slippageTolerance": b.slippage_bps.unwrap_or(100).min(300),
        "originAsset": b.from,
        "depositType": "ORIGIN_CHAIN",
        "destinationAsset": b.to,
        "amount": b.amount,
        "refundTo": b.refund_to,
        "refundType": "ORIGIN_CHAIN",
        "recipient": b.recipient,
        "recipientType": "DESTINATION_CHAIN",
        "deadline": format_rfc3339(deadline),
    });
    let r = reqwest::Client::new().post(format!("{ONECLICK}/quote")).json(&req).timeout(Duration::from_secs(25)).send().await.map_err(bad)?;
    let ok = r.status().is_success();
    let v: Value = r.json().await.map_err(bad)?;
    if !ok {
        return Err(bad(v["message"].as_str().unwrap_or("no quote")));
    }
    Ok(Json(json!({
        "quote": signed_quote_json(&v),
        "signature": v["signature"],
        "deposit_address": v["quote"]["depositAddress"],
        "amount_in": v["quote"]["amountIn"],
        "amount_out": v["quote"]["amountOutFormatted"],
        "min_amount_out": v["quote"]["minAmountOut"],
        "amount_in_usd": v["quote"]["amountInUsd"],
        "amount_out_usd": v["quote"]["amountOutUsd"],
        "deadline": v["quote"]["deadline"],
        "time_estimate_s": v["quote"]["timeEstimate"],
    })))
}

#[derive(serde::Deserialize)]
pub struct SubmitBody {
    tx_hash: String,
    deposit_address: String,
}

/// Tells 1Click about the deposit transaction, so it needn't wait to spot it.
pub async fn intents_submit(Json(b): Json<SubmitBody>) -> Reply {
    if !plain(&b.tx_hash, 128) || !plain(&b.deposit_address, 100) {
        return Err(bad("bad tx hash or address"));
    }
    let r = reqwest::Client::new()
        .post(format!("{ONECLICK}/deposit/submit"))
        .json(&json!({ "txHash": b.tx_hash, "depositAddress": b.deposit_address }))
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(bad)?;
    let v: Value = r.json().await.map_err(bad)?;
    Ok(Json(json!({ "status": v["status"] })))
}

#[derive(serde::Deserialize)]
pub struct StatusQuery {
    deposit_address: String,
}

pub async fn intents_status(axum::extract::Query(q): axum::extract::Query<StatusQuery>) -> Reply {
    if !plain(&q.deposit_address, 100) {
        return Err(bad("bad address"));
    }
    let r = reqwest::Client::new()
        .get(format!("{ONECLICK}/status?depositAddress={}", q.deposit_address))
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(bad)?;
    let v: Value = r.json().await.map_err(bad)?;
    let d = &v["swapDetails"];
    Ok(Json(json!({
        "status": v["status"], "updated_at": v["updatedAt"],
        "amount_out": d["amountOutFormatted"], "destination_tx": d["destinationChainTxHashes"], "refunded": d["refundedAmountFormatted"],
    })))
}

/// UTC seconds since 1970 as RFC 3339 (civil-from-days, H. Hinnant).
fn format_rfc3339(secs: u64) -> String {
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    #[test]
    fn signed_quote_json_is_what_1click_signed() {
        // A mainnet quote (13 November 2026) and the JSON that verifies
        // against 1Click's key (the extension's intents-self.json).
        let raw: serde_json::Value = serde_json::from_str(include_str!("testdata/intents-raw.json")).unwrap();
        let fixture: serde_json::Value = serde_json::from_str(include_str!("../../extension/src/testdata/intents-self.json")).unwrap();
        assert_eq!(super::signed_quote_json(&raw), fixture["quote"].as_str().unwrap());
    }

    #[test]
    fn tls_config_names_its_provider() {
        // With two providers linked, the default builder can't choose.
        assert!(std::panic::catch_unwind(|| rustls::ClientConfig::builder()).is_err());
        let c = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() })
            .with_no_client_auth();
        assert!(!c.alpn_protocols.iter().any(|p| p.is_empty()));
    }

    #[test]
    fn rfc3339() {
        assert_eq!(super::format_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(super::format_rfc3339(1_794_196_541), "2026-11-09T03:55:41Z");
        assert_eq!(super::format_rfc3339(951_782_400), "2000-02-29T00:00:00Z");
    }
}
