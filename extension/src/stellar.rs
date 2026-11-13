//! Stellar transactions for Chain Signatures (ed25519, path `stellar`).
//!
//! The agent sends typed operations; the extension builds the transaction's
//! XDR itself and signs its hash: sha256(sha256(network passphrase) ||
//! ENVELOPE_TYPE_TX || tx). Stellar's order book, path payments and AMM
//! pools are native operations, so every rule is checked on fields the
//! extension encodes. Liquidity pool IDs are computed here from the two
//! assets, so the agent can't name some other pool.
//!
//! Layout checked against @stellar/stellar-sdk 13's XDR.

use near_sdk::{env, json_types::U64, near};

use crate::rules::StellarGrant;

pub const TESTNET: &str = "Test SDF Network ; September 2015";
/// Highest fee per transaction: 0.01 XLM.
pub const MAX_FEE: u32 = 100_000;
/// The pool fee Stellar's constant-product pools use, in basis points.
const POOL_FEE_BP: i32 = 30;

#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SAsset {
    /// "XLM" for lumens, else a 1–12 character code.
    pub code: String,
    #[serde(default)]
    pub issuer: Option<String>,
}

#[near(serializers = [json])]
#[derive(Clone, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StellarOp {
    Payment { destination: String, asset: SAsset, amount: U64 },
    /// Trust line to a listed asset.
    ChangeTrust { asset: SAsset, limit: U64 },
    /// Trust line to the pool share of two listed assets (no limit).
    PoolTrust { asset_a: SAsset, asset_b: SAsset },
    /// Order book: sell `amount` of `selling` at `price_n/price_d` of `buying`.
    SellOffer { selling: SAsset, buying: SAsset, amount: U64, price_n: i32, price_d: i32, #[serde(default)] offer_id: U64 },
    /// Swap through the order book and pools, to the account itself.
    SwapSend { send_asset: SAsset, send_amount: U64, dest_asset: SAsset, dest_min: U64, #[serde(default)] path: Vec<SAsset> },
    /// AMM deposit into the pool of two listed assets.
    PoolDeposit { asset_a: SAsset, asset_b: SAsset, max_a: U64, max_b: U64, min_price: (i32, i32), max_price: (i32, i32) },
    /// AMM withdraw: pool shares back into the two assets.
    PoolWithdraw { asset_a: SAsset, asset_b: SAsset, amount: U64, min_a: U64, min_b: U64 },
    /// Blend lending (a Soroban contract): `submit(account, account,
    /// account, requests)` on a listed pool. The only operation in its
    /// transaction; needs `soroban_data`.
    BlendSubmit { pool: String, requests: Vec<BlendRequest> },
}

/// One Blend request: `kind` supply, withdraw, supply_collateral,
/// withdraw_collateral, borrow or repay; `asset` the reserve's token
/// contract (C…); `amount` in its smallest unit.
#[near(serializers = [json])]
#[derive(Clone, Debug)]
pub struct BlendRequest {
    pub kind: String,
    pub asset: String,
    pub amount: U64,
}

#[near(serializers = [json])]
#[derive(Clone, Debug)]
pub struct StellarTx {
    /// The account's sequence number plus one.
    pub seq_num: U64,
    /// Total fee in stroops.
    pub fee: u32,
    /// Unix seconds after which the transaction is invalid.
    pub max_time: U64,
    pub ops: Vec<StellarOp>,
    /// For a Soroban call: the SorobanTransactionData XDR (hex) from
    /// simulating it. It sets resource limits and the resource fee; it can't
    /// change what the call does (a wrong one only makes it fail). The fee
    /// above must cover it and is counted.
    #[serde(default)]
    pub soroban_data: Option<String>,
}

/// Highest fee for a Soroban transaction (resource fee included): 2 XLM.
pub const MAX_SOROBAN_FEE: u32 = 20_000_000;

/// A C… contract address as its 32-byte ID.
pub fn contract(a: &str) -> Result<[u8; 32], &'static str> {
    let b = base32(a).ok_or("not a Stellar contract")?;
    if b.len() != 35 || b[0] != 2 << 3 || b[33..] != crc16(&b[..33]).to_le_bytes() {
        return Err("not a Stellar contract");
    }
    Ok(b[1..33].try_into().unwrap())
}

/// The native XLM token contract's ID: sha256(HashIDPreimage
/// ENVELOPE_TYPE_CONTRACT_ID { network ID, FROM_ASSET native }).
pub fn xlm_contract(network: &str) -> [u8; 32] {
    let mut p = 8u32.to_be_bytes().to_vec();
    p.extend(env::sha256_array(network.as_bytes()));
    p.extend(1u32.to_be_bytes()); // CONTRACT_ID_PREIMAGE_FROM_ASSET
    p.extend(0u32.to_be_bytes()); // ASSET_TYPE_NATIVE
    env::sha256_array(&p)
}

fn sc_string(o: &mut Vec<u8>, s: &str) {
    u32b(o, s.len() as u32);
    o.extend(s.as_bytes());
    o.extend(vec![0u8; (4 - s.len() % 4) % 4]);
}
fn sc_symbol(o: &mut Vec<u8>, s: &str) {
    u32b(o, 15); // SCV_SYMBOL
    sc_string(o, s);
}
fn sc_account(o: &mut Vec<u8>, k: &[u8; 32]) {
    u32b(o, 18); // SCV_ADDRESS
    u32b(o, 0); // SC_ADDRESS_TYPE_ACCOUNT
    key(o, k);
}
fn sc_contract(o: &mut Vec<u8>, c: &[u8; 32]) {
    u32b(o, 18); // SCV_ADDRESS
    u32b(o, 1); // SC_ADDRESS_TYPE_CONTRACT
    o.extend(c);
}
fn sc_i128(o: &mut Vec<u8>, v: u64) {
    u32b(o, 10); // SCV_I128: hi, lo
    o.extend(0i64.to_be_bytes());
    o.extend(v.to_be_bytes());
}

/// InvokeContractArgs: contract, function name, arguments (already encoded).
fn invoke_args(o: &mut Vec<u8>, c: &[u8; 32], f: &str, args: &[Vec<u8>]) {
    u32b(o, 1); // SC_ADDRESS_TYPE_CONTRACT
    o.extend(c);
    sc_string(o, f);
    u32b(o, args.len() as u32);
    for a in args {
        o.extend(a);
    }
}

/// Blend's request types (pool/src/pool/actions.rs, RequestType).
fn blend_kind(k: &str) -> Result<u32, &'static str> {
    Ok(match k {
        "supply" => 0,
        "withdraw" => 1,
        "supply_collateral" => 2,
        "withdraw_collateral" => 3,
        "borrow" => 4,
        "repay" => 5,
        _ => return Err("unknown Blend request"),
    })
}

/// The InvokeHostFunction operation body for a Blend submit, with its one
/// authorization entry (source-account credentials): the submit call and,
/// under it, one token transfer from the account to the pool per asset the
/// pool pulls (supply, collateral, repay), in the order the pool makes them
/// (its transfers map, ascending by token address). Returns the spends too.
fn blend_op(o: &mut Vec<u8>, me: &[u8; 32], pool: &[u8; 32], requests: &[BlendRequest], network: &str) -> Result<Vec<(String, u128)>, &'static str> {
    if requests.is_empty() || requests.len() > 4 {
        return Err("1 to 4 Blend requests");
    }
    let xlm = xlm_contract(network);
    let name = |c: &[u8; 32]| if *c == xlm { "xlm".to_string() } else { format!("soroban:{}", contract_strkey(c)) };
    let mut spends: Vec<(String, u128)> = vec![];
    let mut pulls: Vec<([u8; 32], u64)> = vec![];
    let mut reqs = vec![];
    for r in requests {
        let k = blend_kind(&r.kind)?;
        let a = contract(&r.asset)?;
        if r.amount.0 == 0 || r.amount.0 > i64::MAX as u64 {
            return Err("amount out of range");
        }
        match k {
            0 | 2 | 5 => {
                match pulls.iter_mut().find(|(c, _)| *c == a) {
                    Some((_, v)) => *v = v.checked_add(r.amount.0).ok_or("overflow")?,
                    None => pulls.push((a, r.amount.0)),
                }
                spends.push((name(&a), r.amount.0 as u128));
            }
            4 => spends.push((format!("borrow:stellar:{}", contract_strkey(&a)), r.amount.0 as u128)),
            _ => {} // withdrawals: to the account itself
        }
        // Request { address, amount, request_type }: a map, keys sorted.
        let mut m = vec![];
        u32b(&mut m, 17); // SCV_MAP
        u32b(&mut m, 1);
        u32b(&mut m, 3);
        sc_symbol(&mut m, "address");
        sc_contract(&mut m, &a);
        sc_symbol(&mut m, "amount");
        sc_i128(&mut m, r.amount.0);
        sc_symbol(&mut m, "request_type");
        u32b(&mut m, 3); // SCV_U32
        u32b(&mut m, k);
        reqs.push(m);
    }
    pulls.sort_by(|x, y| x.0.cmp(&y.0));
    let mut account = vec![];
    sc_account(&mut account, me);
    let mut vec = vec![];
    u32b(&mut vec, 16); // SCV_VEC
    u32b(&mut vec, 1);
    u32b(&mut vec, reqs.len() as u32);
    for r in &reqs {
        vec.extend(r);
    }
    let args = [account.clone(), account.clone(), account.clone(), vec];
    u32b(o, 24); // INVOKE_HOST_FUNCTION
    u32b(o, 0); // HOST_FUNCTION_TYPE_INVOKE_CONTRACT
    invoke_args(o, pool, "submit", &args);
    u32b(o, 1); // one authorization entry
    u32b(o, 0); // SOROBAN_CREDENTIALS_SOURCE_ACCOUNT
    u32b(o, 0); // SOROBAN_AUTHORIZED_FUNCTION_TYPE_CONTRACT_FN
    invoke_args(o, pool, "submit", &args);
    u32b(o, pulls.len() as u32);
    for (token, amount) in &pulls {
        let mut to = vec![];
        sc_contract(&mut to, pool);
        let mut amt = vec![];
        sc_i128(&mut amt, *amount);
        u32b(o, 0);
        invoke_args(o, token, "transfer", &[account.clone(), to, amt]);
        u32b(o, 0); // no deeper invocations
    }
    Ok(spends)
}

/// A contract ID as its C… address.
pub fn contract_strkey(c: &[u8; 32]) -> String {
    let mut b = vec![2u8 << 3];
    b.extend(c);
    b.extend(crc16(&b).to_le_bytes());
    const A: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let (mut out, mut buf, mut bits) = (String::new(), 0u32, 0u32);
    for x in b {
        buf = buf << 8 | x as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(A[((buf >> bits) & 31) as usize] as char);
        }
        buf &= (1 << bits) - 1;
    }
    if bits > 0 {
        out.push(A[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// A G… account address as its ed25519 key.
pub fn account(a: &str) -> Result<[u8; 32], &'static str> {
    let b = base32(a).ok_or("not a Stellar account")?;
    if b.len() != 35 || b[0] != 6 << 3 {
        return Err("not a Stellar account");
    }
    let crc = crc16(&b[..33]);
    if b[33..] != crc.to_le_bytes() {
        return Err("not a Stellar account");
    }
    Ok(b[1..33].try_into().unwrap())
}

fn base32(s: &str) -> Option<Vec<u8>> {
    let (mut buf, mut bits, mut out) = (0u32, 0u32, vec![]);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'2'..=b'7' => c - b'2' + 26,
            _ => return None,
        } as u32;
        buf = buf << 5 | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// CRC16-XModem, as Stellar's StrKey uses.
fn crc16(b: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &x in b {
        crc ^= (x as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { crc << 1 ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

/// The cap asset name: `xlm`, or `stellar:<code>:<issuer>`.
pub fn asset_name(a: &SAsset) -> String {
    match &a.issuer {
        None => "xlm".into(),
        Some(i) => format!("stellar:{}:{i}", a.code),
    }
}

fn u32b(o: &mut Vec<u8>, v: u32) { o.extend(v.to_be_bytes()) }
fn i64b(o: &mut Vec<u8>, v: u64) -> Result<(), &'static str> {
    if v > i64::MAX as u64 {
        return Err("amount too large");
    }
    o.extend(v.to_be_bytes());
    Ok(())
}
fn key(o: &mut Vec<u8>, k: &[u8; 32]) {
    u32b(o, 0); // KEY_TYPE_ED25519
    o.extend(k);
}

fn asset(o: &mut Vec<u8>, a: &SAsset) -> Result<(), &'static str> {
    match &a.issuer {
        None if a.code == "XLM" => u32b(o, 0),
        None => return Err("issued assets need an issuer"),
        Some(i) => {
            let c = a.code.as_bytes();
            if c.is_empty() || c.len() > 12 || !c.iter().all(|b| b.is_ascii_alphanumeric()) {
                return Err("bad asset code");
            }
            let n = if c.len() <= 4 { 4 } else { 12 };
            u32b(o, if n == 4 { 1 } else { 2 });
            let mut code = c.to_vec();
            code.resize(n, 0);
            o.extend(code);
            key(o, &account(i)?);
        }
    }
    Ok(())
}

fn pool_params(a: &SAsset, b: &SAsset) -> Result<Vec<u8>, &'static str> {
    let mut p = vec![];
    u32b(&mut p, 0); // LIQUIDITY_POOL_CONSTANT_PRODUCT
    asset(&mut p, a)?;
    asset(&mut p, b)?;
    p.extend(POOL_FEE_BP.to_be_bytes());
    Ok(p)
}

/// A pool's ID: sha256 of its parameters' XDR.
pub fn pool_id(a: &SAsset, b: &SAsset) -> Result<[u8; 32], &'static str> {
    Ok(env::sha256_array(&pool_params(a, b)?))
}

fn price(o: &mut Vec<u8>, (n, d): (i32, i32)) -> Result<(), &'static str> {
    if n <= 0 || d <= 0 {
        return Err("prices must be positive");
    }
    o.extend(n.to_be_bytes());
    o.extend(d.to_be_bytes());
    Ok(())
}

/// Checks the transaction; returns its XDR, the hash to sign and the spends.
pub fn check(g: &StellarGrant, tx: &StellarTx, queued: bool, network: &str) -> Result<(Vec<u8>, [u8; 32], Vec<(String, u128)>), &'static str> {
    let me = account(&g.self_address)?;
    if tx.ops.is_empty() || tx.ops.len() > 8 {
        return Err("1 to 8 operations");
    }
    let soroban = tx.ops.iter().any(|o| matches!(o, StellarOp::BlendSubmit { .. }));
    if soroban {
        if tx.ops.len() != 1 {
            return Err("a Soroban call must be the only operation");
        }
        if tx.soroban_data.is_none() {
            return Err("a Soroban call needs its resources (soroban_data)");
        }
        if tx.fee == 0 || tx.fee > MAX_SOROBAN_FEE {
            return Err("fee outside 1 stroop to 2 XLM");
        }
    } else {
        if tx.soroban_data.is_some() {
            return Err("soroban_data without a Soroban call");
        }
        if tx.fee == 0 || tx.fee > MAX_FEE {
            return Err("fee outside 1 stroop to 0.01 XLM");
        }
    }
    let allowed = |a: &SAsset| match &a.issuer {
        None => a.code == "XLM",
        Some(i) => g.assets.iter().any(|x| *x == format!("{}:{i}", a.code)),
    };
    let need = |a: &SAsset| if allowed(a) { Ok(()) } else { Err("asset not allowed") };
    let mut spends: Vec<(String, u128)> = vec![("xlm".into(), tx.fee as u128)];
    let add = |s: &mut Vec<(String, u128)>, a: &SAsset, v: u64| {
        let n = asset_name(a);
        match s.iter_mut().find(|(x, _)| *x == n) {
            Some((_, t)) => *t += v as u128,
            None => s.push((n, v as u128)),
        }
    };
    let mut o = vec![];
    key(&mut o, &me); // source (MuxedAccount, ed25519)
    u32b(&mut o, tx.fee);
    i64b(&mut o, tx.seq_num.0)?;
    u32b(&mut o, 1); // PRECOND_TIME
    o.extend(0u64.to_be_bytes());
    o.extend(tx.max_time.0.to_be_bytes());
    u32b(&mut o, 0); // MEMO_NONE
    u32b(&mut o, tx.ops.len() as u32);
    for op in &tx.ops {
        u32b(&mut o, 0); // no per-operation source: the account itself
        match op {
            StellarOp::Payment { destination, asset: a, amount } => {
                need(a)?;
                let to = account(destination)?;
                if to == me {
                    return Err("payment to itself");
                }
                if !queued && !g.recipients.contains(destination) {
                    return Err("recipient not allowed");
                }
                u32b(&mut o, 1);
                key(&mut o, &to);
                asset(&mut o, a)?;
                i64b(&mut o, amount.0)?;
                add(&mut spends, a, amount.0);
            }
            StellarOp::ChangeTrust { asset: a, limit } => {
                if a.issuer.is_none() {
                    return Err("no trust line for XLM");
                }
                need(a)?;
                u32b(&mut o, 6);
                asset(&mut o, a)?;
                i64b(&mut o, limit.0)?;
            }
            StellarOp::PoolTrust { asset_a, asset_b } => {
                if !g.amm {
                    return Err("AMMs not allowed");
                }
                need(asset_a)?;
                need(asset_b)?;
                u32b(&mut o, 6);
                u32b(&mut o, 3); // ASSET_TYPE_POOL_SHARE
                o.extend(pool_params(asset_a, asset_b)?);
                i64b(&mut o, i64::MAX as u64)?;
            }
            StellarOp::SellOffer { selling, buying, amount, price_n, price_d, offer_id } => {
                if !g.dex {
                    return Err("order book not allowed");
                }
                need(selling)?;
                need(buying)?;
                u32b(&mut o, 3);
                asset(&mut o, selling)?;
                asset(&mut o, buying)?;
                i64b(&mut o, amount.0)?;
                price(&mut o, (*price_n, *price_d))?;
                i64b(&mut o, offer_id.0)?;
                add(&mut spends, selling, amount.0);
            }
            StellarOp::SwapSend { send_asset, send_amount, dest_asset, dest_min, path } => {
                if !g.dex {
                    return Err("order book not allowed");
                }
                need(send_asset)?;
                need(dest_asset)?;
                if path.len() > 5 {
                    return Err("path too long");
                }
                for p in path {
                    need(p)?;
                }
                if dest_min.0 == 0 {
                    return Err("set a minimum to receive");
                }
                u32b(&mut o, 13);
                asset(&mut o, send_asset)?;
                i64b(&mut o, send_amount.0)?;
                key(&mut o, &me); // the result comes back to the account
                asset(&mut o, dest_asset)?;
                i64b(&mut o, dest_min.0)?;
                u32b(&mut o, path.len() as u32);
                for p in path {
                    asset(&mut o, p)?;
                }
                add(&mut spends, send_asset, send_amount.0);
            }
            StellarOp::PoolDeposit { asset_a, asset_b, max_a, max_b, min_price, max_price } => {
                if !g.amm {
                    return Err("AMMs not allowed");
                }
                need(asset_a)?;
                need(asset_b)?;
                u32b(&mut o, 22);
                o.extend(pool_id(asset_a, asset_b)?);
                i64b(&mut o, max_a.0)?;
                i64b(&mut o, max_b.0)?;
                price(&mut o, *min_price)?;
                price(&mut o, *max_price)?;
                add(&mut spends, asset_a, max_a.0);
                add(&mut spends, asset_b, max_b.0);
            }
            StellarOp::BlendSubmit { pool, requests } => {
                let p = contract(pool)?;
                if !queued && !g.blend.contains(pool) {
                    return Err("not an allowed lending pool");
                }
                // The op source written above (none) is part of the op; the
                // body follows.
                for (a, v) in blend_op(&mut o, &me, &p, requests, network)? {
                    match spends.iter_mut().find(|(x, _)| *x == a) {
                        Some((_, t)) => *t += v,
                        None => spends.push((a, v)),
                    }
                }
            }
            StellarOp::PoolWithdraw { asset_a, asset_b, amount, min_a, min_b } => {
                if !g.amm {
                    return Err("AMMs not allowed");
                }
                need(asset_a)?;
                need(asset_b)?;
                u32b(&mut o, 23);
                o.extend(pool_id(asset_a, asset_b)?);
                i64b(&mut o, amount.0)?;
                i64b(&mut o, min_a.0)?;
                i64b(&mut o, min_b.0)?;
            }
        }
    }
    match &tx.soroban_data {
        None => u32b(&mut o, 0), // ext v0
        Some(d) => {
            let d = hex::decode(d).map_err(|_| "soroban_data must be hex")?;
            if d.len() < 32 || d.len() > 8192 {
                return Err("soroban_data has the wrong size");
            }
            // SorobanTransactionData ends with the resource fee (int64),
            // which the transaction fee must cover.
            let rf = i64::from_be_bytes(d[d.len() - 8..].try_into().unwrap());
            if rf < 0 || rf as u64 >= tx.fee as u64 {
                return Err("the fee doesn't cover the resource fee");
            }
            u32b(&mut o, 1); // ext v1
            o.extend(d);
        }
    }
    let mut payload = env::sha256_array(network.as_bytes()).to_vec();
    payload.extend(2u32.to_be_bytes()); // ENVELOPE_TYPE_TX
    payload.extend(&o);
    let hash = env::sha256_array(&payload);
    Ok((o, hash, spends))
}
