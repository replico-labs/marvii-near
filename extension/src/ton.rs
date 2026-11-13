//! TON for Chain Signatures (ed25519, path `ton`): the account is a wallet
//! v4r2 whose key is the MPC key. The agent sends typed messages; the
//! extension builds every cell itself and has the key sign the hash of the
//! wallet's signing message (ton_cell.rs). Each internal message goes with
//! send mode 3, never 128 (whole balance) or 32 (destroy).
//!
//! - `pay`: TON to a listed recipient.
//! - `jetton`: a TEP-74 transfer from the account's own jetton wallet (the
//!   pairing of master and wallet is on the passkey-approved grant), to a
//!   listed recipient; excess TON comes back to the account.
//! - `evaa_supply` / `evaa_withdraw`: EVAA lending (layouts from
//!   @evaafi/sdk 1.0.3, AbstractMaster and ClassicMaster, for master code
//!   version 0, the version the testnet master reports). Supply credits the
//!   account; withdraw pays only the account. A withdraw past what was
//!   supplied is a borrow, and the extension can't tell which, so every
//!   withdraw counts against `borrow:ton:<asset id>`. Repaying is supplying.

use near_sdk::{json_types::{U128, U64}, near};

use crate::rules::TonGrant;
use crate::ton_cell::{self, Addr, Cell};

/// TEP-74 jetton transfer, as in @evaafi/sdk OPCODES.JETTON_TRANSFER.
const OP_JETTON_TRANSFER: u128 = 0x0f8a7ea5;
/// @evaafi/sdk OPCODES: SUPPLY_MASTER, SUPPLY_WITHDRAW_MASTER.
const OP_EVAA_SUPPLY: u128 = 0x1;
const OP_EVAA_SUPPLY_WITHDRAW: u128 = 0x4;
/// SUPPLY_WITHDRAW_MASTER_WITHOUT_PRICES: the same body with no price cell
/// (a withdrawal the master can settle without prices: no debt).
const OP_EVAA_SUPPLY_WITHDRAW_NO_PRICES: u128 = 0x401;
/// TON attached to a contract call for its gas, at most (counted).
pub const MAX_GAS: u64 = 1_000_000_000;

#[near(serializers = [json])]
#[derive(Clone, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TonMsg {
    /// Pay TON (nanotons), with an optional text comment.
    Pay { to: String, amount: U128, #[serde(default)] comment: Option<String> },
    /// Send jettons: `jetton` is the master; `ton` pays the transfer's gas.
    Jetton { jetton: String, to: String, amount: U128, ton: U64, #[serde(default)] forward_ton: U64 },
    /// Supply to EVAA: TON (`asset` "ton"), or a jetton by its master.
    EvaaSupply { master: String, asset: String, amount: U128, ton: U64, #[serde(default)] forward_ton: U64 },
    /// Withdraw (or borrow) from EVAA: `asset_id` is EVAA's 256-bit id
    /// (hex); `prices` the oracle-signed price cell (BOC hex), which the
    /// master checks, or empty for a withdrawal without prices.
    EvaaWithdraw { master: String, asset_id: String, amount: U64, prices: String, ton: U64 },
}

#[near(serializers = [json])]
#[derive(Clone, Debug)]
pub struct TonTx {
    pub seqno: u32,
    pub valid_until: u32,
    pub messages: Vec<TonMsg>,
}

pub fn address(s: &str) -> Result<Addr, &'static str> {
    let (a, _) = ton_cell::parse_address(s)?;
    if a.wc != 0 {
        return Err("only workchain 0");
    }
    Ok(a)
}

fn key(a: &Addr) -> String {
    ton_cell::friendly(a, true, true)
}

fn gas(ton: u64) -> Result<u128, &'static str> {
    if ton == 0 || ton > MAX_GAS {
        return Err("gas must be 1 nanoton to 1 TON");
    }
    Ok(ton as u128)
}

/// Checks the transaction; returns the signing message, its hash (what the
/// key signs) and the spends.
pub fn check(g: &TonGrant, tx: &TonTx, queued: bool) -> Result<(Cell, [u8; 32], Vec<(String, u128)>), &'static str> {
    let me = address(&g.self_address)?;
    if tx.messages.is_empty() || tx.messages.len() > 4 {
        return Err("1 to 4 messages");
    }
    let listed = |a: &Addr| g.recipients.iter().any(|r| address(r).ok() == Some(*a));
    let evaa = |a: &Addr| g.evaa.iter().any(|r| address(r).ok() == Some(*a));
    let jetton_wallet = |master: &str| -> Result<(Addr, Addr), &'static str> {
        let m = address(master)?;
        let j = g.jettons.iter().find(|j| address(&j.master).ok() == Some(m)).ok_or("jetton not allowed")?;
        Ok((m, address(&j.wallet)?))
    };
    let query = tx.seqno as u128;
    let mut spends: Vec<(String, u128)> = vec![];
    let mut msgs = vec![];
    for m in &tx.messages {
        let msg = match m {
            TonMsg::Pay { to, amount, comment } => {
                let (to, bounce) = ton_cell::parse_address(to)?;
                if to.wc != 0 {
                    return Err("only workchain 0");
                }
                if to == me {
                    return Err("payment to itself");
                }
                if !queued && !listed(&to) {
                    return Err("recipient not allowed");
                }
                if amount.0 == 0 {
                    return Err("amount must be more than zero");
                }
                spends.push(("ton".into(), amount.0));
                let body = match comment {
                    Some(c) if !c.is_empty() => Some(ton_cell::comment(c)?),
                    _ => None,
                };
                ton_cell::internal_message(&to, amount.0, bounce, body)?
            }
            TonMsg::Jetton { jetton, to, amount, ton, forward_ton } => {
                let (master, wallet) = jetton_wallet(jetton)?;
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
                let ton = gas(ton.0)?;
                if forward_ton.0 as u128 >= ton {
                    return Err("the forward amount must be less than the gas sent");
                }
                spends.push((format!("jetton:{}", key(&master)), amount.0));
                spends.push(("ton".into(), ton));
                let body = jetton_transfer(query, amount.0, &to, &me, forward_ton.0, None)?;
                ton_cell::internal_message(&wallet, ton, true, Some(body))?
            }
            TonMsg::EvaaSupply { master, asset, amount, ton, forward_ton } => {
                let pool = address(master)?;
                if !queued && !evaa(&pool) {
                    return Err("not an allowed lending pool");
                }
                if amount.0 == 0 {
                    return Err("amount must be more than zero");
                }
                let ton = gas(ton.0)?;
                if asset == "ton" {
                    let amount: u64 = amount.0.try_into().map_err(|_| "amount too large")?;
                    spends.push(("ton".into(), amount as u128 + ton));
                    let body = evaa_supply(Some((query, amount)), &me)?;
                    ton_cell::internal_message(&pool, amount as u128 + ton, true, Some(body))?
                } else {
                    let (jm, wallet) = jetton_wallet(asset)?;
                    if forward_ton.0 == 0 || forward_ton.0 as u128 >= ton {
                        return Err("the forward amount must be more than zero and less than the gas sent");
                    }
                    spends.push((format!("jetton:{}", key(&jm)), amount.0));
                    spends.push(("ton".into(), ton));
                    let body = jetton_transfer(query, amount.0, &pool, &me, forward_ton.0, Some(evaa_supply(None, &me)?))?;
                    ton_cell::internal_message(&wallet, ton, true, Some(body))?
                }
            }
            TonMsg::EvaaWithdraw { master, asset_id, amount, prices, ton } => {
                let pool = address(master)?;
                if !queued && !evaa(&pool) {
                    return Err("not an allowed lending pool");
                }
                let id: [u8; 32] = hex::decode(asset_id).map_err(|_| "asset id must be 32 bytes of hex")?.try_into().map_err(|_| "asset id must be 32 bytes of hex")?;
                if amount.0 == 0 {
                    return Err("amount must be more than zero");
                }
                let prices = if prices.is_empty() { None } else { Some(Cell::from_boc(&hex::decode(prices).map_err(|_| "prices must be BOC hex")?)?) };
                let ton = gas(ton.0)?;
                spends.push((format!("borrow:ton:{}", hex::encode(id)), amount.0 as u128));
                spends.push(("ton".into(), ton));
                let body = evaa_withdraw(query, &id, amount.0, &me, prices)?;
                ton_cell::internal_message(&pool, ton, true, Some(body))?
            }
        };
        msgs.push(msg);
    }
    let signing = ton_cell::v4_signing_message(tx.seqno, tx.valid_until, msgs)?;
    // Merge spends of the same asset.
    let mut merged: Vec<(String, u128)> = vec![];
    for (a, v) in spends {
        match merged.iter_mut().find(|(x, _)| *x == a) {
            Some((_, t)) => *t = t.checked_add(v).ok_or("overflow")?,
            None => merged.push((a, v)),
        }
    }
    let hash = signing.hash();
    Ok((signing, hash, merged))
}

/// TEP-74 transfer: op, query id, amount, destination, response
/// destination (the account, for excess TON), no custom payload, forward
/// TON, and the forward payload in a reference or empty.
pub(crate) fn jetton_transfer(query: u128, amount: u128, to: &Addr, me: &Addr, forward_ton: u64, forward: Option<Cell>) -> Result<Cell, &'static str> {
    let mut c = Cell::new();
    c.uint(OP_JETTON_TRANSFER, 32).uint(query, 64).coins(amount).addr(Some(to)).addr(Some(me)).bit(false).coins(forward_ton as u128);
    match forward {
        Some(f) => c.bit(true).push_ref(f),
        None => c.bit(false),
    };
    c.done()
}

/// EVAA supply (createSupplyMessage): op 1, then for TON the query id; a
/// 2-bit "include user code" (yes, -1); for TON the amount; the user
/// credited (the account); an empty payload; subaccount 0; no
/// repay-remainings return; the custom payload recipient (the account); no
/// saturation. The testnet masters fail (exit code 9) on an empty
/// recipient; every supply they accepted named the user and set -1.
pub(crate) fn evaa_supply(ton: Option<(u128, u64)>, me: &Addr) -> Result<Cell, &'static str> {
    let mut c = Cell::new();
    c.uint(OP_EVAA_SUPPLY, 32);
    if let Some((q, _)) = ton {
        c.uint(q, 64);
    }
    c.uint(0b11, 2);
    if let Some((_, amount)) = ton {
        c.uint(amount as u128, 64);
    }
    c.addr(Some(me)).push_ref(Cell::new()).uint(0, 16).uint(0, 2).addr(Some(me)).uint(0, 2);
    c.done()
}

/// EVAA supply-withdraw with nothing supplied (ClassicMaster.sendWithdraw;
/// without prices, the Pyth master's body too: its empty ref-asset
/// dictionary is the same single 0 bit as an absent price cell):
/// op 4, query id; refs: the supply amount (0), the withdrawal (amount,
/// asset id, recipient: the account), and the general data (no user code,
/// the prices, no TON for repay remainings, an empty payload, subaccount 0,
/// two flags off).
pub(crate) fn evaa_withdraw(query: u128, asset: &[u8; 32], amount: u64, me: &Addr, prices: Option<Cell>) -> Result<Cell, &'static str> {
    let op = if prices.is_some() { OP_EVAA_SUPPLY_WITHDRAW } else { OP_EVAA_SUPPLY_WITHDRAW_NO_PRICES };
    let supply = Cell::new().uint(0, 64).done()?;
    let withdraw = Cell::new().uint(amount as u128, 64).bytes(asset).addr(Some(me)).done()?;
    let general = Cell::new().uint(0, 2).maybe_ref(prices).uint(0, 64).push_ref(Cell::new()).uint(0, 16).uint(0, 2).uint(0, 2).done()?;
    Cell::new().uint(op, 32).uint(query, 64).push_ref(supply).push_ref(withdraw).push_ref(general).done()
}
