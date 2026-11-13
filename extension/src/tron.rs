//! TRON transactions for Chain Signatures (secp256k1, path `tron`).
//!
//! The agent sends typed fields; the extension builds the transaction's
//! `raw_data` protobuf itself and signs sha256(raw_data), the transaction ID.
//! TRON contracts take the same call data as EVM ones, so TRC-20 transfers
//! and approvals reuse the EVM decoder; venue calls with a known layout are
//! checked to send their result to the account:
//!
//! - SunSwap V2 (a Uniswap V2 fork): `to` must be the account;
//! - JustLend (a Compound fork): every call acts for `msg.sender`, the
//!   account; `borrow` counts against `borrow:tron:<jToken>`;
//! - TRC-721 `transferFrom` / `safeTransferFrom`: only the account's NFTs,
//!   only to listed recipients, only on listed NFT contracts.
//!
//! Selectors computed with viem's toFunctionSelector from the signatures in
//! the deployed contracts' ABIs on Nile.

use near_sdk::{env, json_types::U64, near};

use crate::evm::{self, Decoded};
use crate::rules::TronGrant;

pub const TRANSFER_URL: &[u8] = b"type.googleapis.com/protocol.TransferContract";
pub const TRIGGER_URL: &[u8] = b"type.googleapis.com/protocol.TriggerSmartContract";
/// The most energy a call may burn: 200 TRX, counted against the `trx` cap.
pub const MAX_FEE_LIMIT: u64 = 200_000_000;

const SEL_SWAP_EXACT_TRX_FOR_TOKENS: [u8; 4] = [0x7f, 0xf3, 0x6a, 0xb5];
const SEL_SWAP_EXACT_TOKENS_FOR_TOKENS: [u8; 4] = [0x38, 0xed, 0x17, 0x39];
const SEL_SWAP_EXACT_TOKENS_FOR_TRX: [u8; 4] = [0x18, 0xcb, 0xaf, 0xe5];
const SEL_JTOKEN_BORROW: [u8; 4] = [0xc5, 0xeb, 0xea, 0xec];
const SEL_TRANSFER_FROM: [u8; 4] = [0x23, 0xb8, 0x72, 0xdd];
const SEL_SAFE_TRANSFER_FROM: [u8; 4] = [0x42, 0x84, 0x2e, 0x0e];
const SEL_SET_APPROVAL_FOR_ALL: [u8; 4] = [0xa2, 0x2c, 0xb4, 0x65];

#[near(serializers = [json])]
#[derive(Clone, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TronCall {
    /// Pay TRX.
    Transfer { to: String, amount: U64 },
    /// Call a contract: `data` is 0x-prefixed call data, `call_value` in sun.
    Trigger { contract: String, #[serde(default)] call_value: U64, data: String },
}

#[near(serializers = [json])]
#[derive(Clone, Debug)]
pub struct TronTx {
    /// From the latest block (hex): bytes 6..8 of its number, bytes 8..16 of its hash.
    pub ref_block_bytes: String,
    pub ref_block_hash: String,
    pub expiration: U64,
    pub timestamp: U64,
    /// The most energy fee the call may burn, in sun (calls only).
    #[serde(default)]
    pub fee_limit: U64,
    pub call: TronCall,
}

/// A T… address as its 21 bytes (0x41 || 20).
pub fn address(a: &str) -> Result<[u8; 21], &'static str> {
    let b = bs58::decode(a).into_vec().map_err(|_| "not a TRON address")?;
    if b.len() != 25 || b[0] != 0x41 {
        return Err("not a TRON address");
    }
    if env::sha256_array(&env::sha256_array(&b[..21]))[..4] != b[21..] {
        return Err("not a TRON address");
    }
    Ok(b[..21].try_into().unwrap())
}

pub fn fmt(a: &[u8; 21]) -> String {
    let mut b = a.to_vec();
    b.extend(&env::sha256_array(&env::sha256_array(a))[..4]);
    bs58::encode(b).into_string()
}

fn evm20(a: &[u8; 21]) -> [u8; 20] {
    a[1..].try_into().unwrap()
}
fn from_evm(a: &[u8; 20]) -> [u8; 21] {
    let mut o = [0x41u8; 21];
    o[1..].copy_from_slice(a);
    o
}

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
    pb_varint(out, (field as u64) << 3 | 2);
    pb_varint(out, b.len() as u64);
    out.extend_from_slice(b);
}
fn pb_uint(out: &mut Vec<u8>, field: u8, v: u64) {
    if v != 0 {
        pb_varint(out, (field as u64) << 3);
        pb_varint(out, v);
    }
}

/// The address word at `index` of the call's arguments.
fn arg_address(data: &[u8], index: usize) -> Result<[u8; 20], &'static str> {
    let at = 4 + 32 * index;
    evm::word_address(data.get(at..at + 32).ok_or("call data too short")?)
}
fn arg_u128(data: &[u8], index: usize) -> Result<u128, &'static str> {
    let at = 4 + 32 * index;
    evm::word_u128(data.get(at..at + 32).ok_or("call data too short")?)
}

/// Checks the transaction; returns its raw_data, its ID (the hash to sign)
/// and the spends.
pub fn check(g: &TronGrant, tx: &TronTx, queued: bool) -> Result<(Vec<u8>, [u8; 32], Vec<(String, u128)>), &'static str> {
    let me = address(&g.self_address)?;
    let rb = hex::decode(&tx.ref_block_bytes).map_err(|_| "bad block reference")?;
    let rh = hex::decode(&tx.ref_block_hash).map_err(|_| "bad block reference")?;
    if rb.len() != 2 || rh.len() != 8 {
        return Err("bad block reference");
    }
    let listed = |a: &[u8; 21]| g.recipients.iter().any(|r| address(r).ok() == Some(*a));
    let mut spends: Vec<(String, u128)> = vec![];
    let (kind, url, body) = match &tx.call {
        TronCall::Transfer { to, amount } => {
            let to = address(to)?;
            if to == me {
                return Err("payment to itself");
            }
            if !queued && !listed(&to) {
                return Err("recipient not allowed");
            }
            if amount.0 == 0 {
                return Err("amount must be more than zero");
            }
            let mut b = vec![];
            pb_bytes(&mut b, 1, &me);
            pb_bytes(&mut b, 2, &to);
            pb_uint(&mut b, 3, amount.0);
            spends.push(("trx".into(), amount.0 as u128));
            (1u64, TRANSFER_URL, b)
        }
        TronCall::Trigger { contract, call_value, data } => {
            let c = address(contract)?;
            if c == me {
                return Err("call to itself");
            }
            if tx.fee_limit.0 == 0 || tx.fee_limit.0 > MAX_FEE_LIMIT {
                return Err("fee limit must be 1 sun to 200 TRX");
            }
            let data = evm::parse_hex(data)?;
            if data.len() < 4 {
                return Err("a call needs a function");
            }
            if call_value.0 > 0 {
                spends.push(("trx".into(), call_value.0 as u128));
            }
            let token = format!("trc20:{}", fmt(&c));
            let sel: [u8; 4] = data[..4].try_into().unwrap();
            let venue = |c: &[u8; 21]| g.venues.iter().any(|v| address(&v.contract).ok() == Some(*c) && v.selector.to_lowercase() == format!("0x{}", hex::encode(sel)));
            let is_nft = g.nfts.iter().any(|n| address(n).ok() == Some(c));
            if is_nft && (sel == SEL_TRANSFER_FROM || sel == SEL_SAFE_TRANSFER_FROM) {
                // TRC-721: the account's own NFT, to a listed recipient.
                if data.len() != 4 + 96 {
                    return Err("transfer data has the wrong length");
                }
                if arg_address(&data, 0)? != evm20(&me) {
                    return Err("only the account's own NFTs");
                }
                let to = from_evm(&arg_address(&data, 1)?);
                if !queued && !listed(&to) {
                    return Err("recipient not allowed");
                }
            } else if is_nft && sel == SEL_SET_APPROVAL_FOR_ALL {
                // Let a marketplace (a venue) list the account's NFTs.
                if data.len() != 4 + 64 {
                    return Err("approval data has the wrong length");
                }
                let op = from_evm(&arg_address(&data, 0)?);
                if !queued && !g.venues.iter().any(|v| address(&v.contract).ok() == Some(op)) {
                    return Err("spender is not a venue");
                }
            } else {
                match evm::decode(&data)? {
                    Decoded::Native => unreachable!(),
                    Decoded::Transfer { recipient, amount } => {
                        if !queued && !listed(&from_evm(&recipient)) {
                            return Err("recipient not allowed");
                        }
                        spends.push((token, amount));
                    }
                    Decoded::Approve { spender, amount } => {
                        let sp = from_evm(&spender);
                        if !queued && !g.venues.iter().any(|v| address(&v.contract).ok() == Some(sp)) {
                            return Err("spender is not a venue");
                        }
                        spends.push((token, amount));
                    }
                    Decoded::Call { selector } => {
                        if !queued && !venue(&c) {
                            return Err("not an allowed venue");
                        }
                        // Where a swap's output goes, for SunSwap's layouts.
                        let to_word = match selector {
                            SEL_SWAP_EXACT_TRX_FOR_TOKENS => Some(2),
                            SEL_SWAP_EXACT_TOKENS_FOR_TOKENS | SEL_SWAP_EXACT_TOKENS_FOR_TRX => Some(3),
                            _ => None,
                        };
                        if let Some(w) = to_word {
                            if arg_address(&data, w)? != evm20(&me) {
                                return Err("venue call sends the result to someone else");
                            }
                        }
                        if selector == SEL_JTOKEN_BORROW {
                            if data.len() != 4 + 32 {
                                return Err("borrow data has the wrong length");
                            }
                            spends.push((format!("borrow:tron:{}", fmt(&c)), arg_u128(&data, 0)?));
                        }
                    }
                }
            }
            let mut b = vec![];
            pb_bytes(&mut b, 1, &me);
            pb_bytes(&mut b, 2, &c);
            pb_uint(&mut b, 3, call_value.0);
            pb_bytes(&mut b, 4, &data);
            spends.push(("trx".into(), tx.fee_limit.0 as u128));
            (31u64, TRIGGER_URL, b)
        }
    };
    let mut any = vec![];
    pb_bytes(&mut any, 1, url);
    pb_bytes(&mut any, 2, &body);
    let mut contract = vec![];
    pb_uint(&mut contract, 1, kind);
    pb_bytes(&mut contract, 2, &any);
    let mut raw = vec![];
    pb_bytes(&mut raw, 1, &rb);
    pb_bytes(&mut raw, 4, &rh);
    pb_uint(&mut raw, 8, tx.expiration.0);
    pb_bytes(&mut raw, 11, &contract);
    pb_uint(&mut raw, 14, tx.timestamp.0);
    if kind == 31 {
        pb_uint(&mut raw, 18, tx.fee_limit.0);
    }
    // Merge spends of the same asset.
    let mut merged: Vec<(String, u128)> = vec![];
    for (a, v) in spends {
        match merged.iter_mut().find(|(x, _)| *x == a) {
            Some((_, t)) => *t += v,
            None => merged.push((a, v)),
        }
    }
    let id = env::sha256_array(&raw);
    Ok((raw, id, merged))
}
