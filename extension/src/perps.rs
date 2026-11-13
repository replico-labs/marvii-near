//! Phoenix perps on Solana mainnet (Ellipsis Labs' "Rise" exchange, program
//! `Etrn…`), set up the way protean-bot's Solana desk uses it: two keys,
//! each with its own powers on-chain.
//!
//! - The account's own key (path `solana`) is the trader accounts'
//!   `authority`: it owns the collateral. Through `sol_tx` the agent may only
//!   wrap USDC into Phoenix collateral and back (Ember), move it between the
//!   account's own Phoenix accounts and token accounts, open an isolated
//!   subaccount, and delegate a subaccount to the account's own trading key
//!   for it (or back to itself). The USDC wrapped counts against
//!   `spl:<USDC>`. It never places orders.
//! - A trading key per subaccount (path `phoenix-<sub>`, also Chain
//!   Signatures) is the subaccount's `position_authority`. Phoenix lets it
//!   trade that subaccount and never withdraw. The extension signs only
//!   immediate-or-cancel orders with a limit price, on markets the grant
//!   lists, up to the grant's size per order; each order's base lots count
//!   against `phoenix:<market>`. A subaccount holds one position, so its
//!   losses stop at what the owner moved into it.
//!
//! Layouts from @ellipsis-labs/rise 0.6.2 (its instruction builders and
//! discriminants, sha256("global:<name>")[..8]); addresses read on mainnet:
//! the global configuration names the perp asset map, the collateral mint
//! (PhUSD) and the arenas, and Ember's state names USDC and PhUSD.

use near_sdk::{json_types::U64, near};

use crate::sol::{b58, fmt, pda, Ix, Own, COMPUTE_BUDGET, TOKEN};

pub const PROGRAM: &str = "EtrnLzgbS7nMMy5fbD42kXiUzGg8XQzJ972Xtk1cjWih";
pub const EMBER: &str = "EMBERpYNE6ehWmXymZZS2skiFmCa9V5dp14e1iduM5qy";
pub const LOG_AUTHORITY: &str = "GdxfTLSsdSY37G6fZoYtdGDSfgFnbT2EmRpuePZxWShS";
pub const GLOBAL_CONFIG: &str = "2zskx2iyCvb6Stg7RBZkt1f6MrF4dpYtMG3yMvKwqtUZ";
pub const PERP_ASSET_MAP: &str = "2nHGAaEw3D5dd4hVueaUNoygkQFmoeKqRQWnSPqSMFUC";
pub const EMBER_STATE: &str = "6ur7v6AXNpnHeEb6xuk7PyezvZ1i5GrgYyWZkNCpzbRz";
pub const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
pub const PHUSD: &str = "PhUsd11YkbjSaWjFncfAAmatntsjx3MgDR9B6g1ks3A";
/// Rent of an isolated subaccount (max_positions 1), lamports: measured on a
/// fork of mainnet (the account paid 2,077,640 with the 5,000 fee).
pub const SUBACCOUNT_RENT: u128 = 2_072_640;
pub const MAX_SUB: u8 = 31;

const PLACE_MARKET_ORDER: [u8; 8] = [0x5a, 0x76, 0xc0, 0xfc, 0xc0, 0x63, 0x27, 0x91];
const DEPOSIT_FUNDS: [u8; 8] = [0xca, 0x27, 0x34, 0xd3, 0x35, 0x14, 0xfa, 0x58];
const WITHDRAW_FUNDS: [u8; 8] = [0xf1, 0x24, 0x1d, 0x6f, 0xd0, 0x1f, 0x68, 0xd9];
const REGISTER_TRADER: [u8; 8] = [0x4b, 0xf3, 0xe0, 0xa7, 0x01, 0x05, 0x33, 0x20];
const DELEGATE_TRADER: [u8; 8] = [0x03, 0xa4, 0xc0, 0x6a, 0x77, 0x5b, 0x7a, 0xe0];
const TRANSFER_COLLATERAL: [u8; 8] = [0x9d, 0xa3, 0x3f, 0x1b, 0xf2, 0x48, 0xfb, 0x61];
const CHILD_TO_PARENT: [u8; 8] = [0x33, 0x64, 0xb1, 0x73, 0x8b, 0x87, 0xf7, 0x8b];
const SYNC_PARENT_TO_CHILD: [u8; 8] = [0xaf, 0x89, 0xd9, 0x0b, 0xeb, 0x27, 0x96, 0x13];
const EMBER_DEPOSIT: [u8; 8] = [242, 35, 198, 137, 82, 225, 242, 182];
const EMBER_WITHDRAW: [u8; 8] = [183, 18, 70, 156, 148, 109, 161, 34];

/// A Phoenix market the trading key may trade, and the most base lots one
/// order may take.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerpsMarket {
    pub market: String,
    pub max_base_lots: U64,
}

/// An isolated subaccount the agent trades through its own trading key.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerpsDesk {
    /// 1 to 31 (0 is the cross-margin account, which only holds collateral).
    pub sub: u8,
    /// The account's key on path `phoenix-<sub>`, base58, as the passkey saw it.
    pub trader_key: String,
    pub markets: Vec<PerpsMarket>,
}

/// The account's trader account for subaccount `sub`, from the bump the agent supplies.
pub fn trader_account(authority: &[u8; 32], sub: u8, bump: u8) -> [u8; 32] {
    pda(&[b"trader", authority, &[0], &[sub]], bump, &b58(PROGRAM).unwrap())
}

fn u64_at(d: &[u8], at: usize) -> Result<u64, &'static str> {
    d.get(at..at + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap())).ok_or("instruction data too short")
}

fn head(ix: &Ix) -> Result<(), &'static str> {
    let a = &ix.accounts;
    if a.len() < 4 || a[0].key != ix.program || fmt(&a[1].key) != LOG_AUTHORITY || fmt(&a[2].key) != GLOBAL_CONFIG {
        return Err("unexpected Phoenix accounts");
    }
    Ok(())
}

/// One instruction the account's own key signs (Phoenix or Ember); returns the spends.
pub fn check_authority(desks: &[PerpsDesk], own: &Own, ix: &Ix) -> Result<Vec<(String, u128)>, &'static str> {
    let a = &ix.accounts;
    let d = &ix.data;
    let key = |i: usize| a.get(i).map(|m| m.key).ok_or("missing account");
    let signer_me = |i: usize| match a.get(i) {
        Some(m) if m.key == own.me && m.signer => Ok(()),
        _ => Err("the account must sign this"),
    };
    let mine = |i: usize| -> Result<(), &'static str> {
        let k = key(i)?;
        if own.traders.iter().any(|(t, _)| *t == k) { Ok(()) } else { Err("not the account's own Phoenix account") }
    };
    let mint_of = |i: usize| -> Result<String, &'static str> { own.mine(&key(i)?).map(|m| fmt(&m)).ok_or("not the account's own token account") };
    let disc: [u8; 8] = d.get(..8).and_then(|b| b.try_into().ok()).ok_or("instruction data too short")?;
    if fmt(&ix.program) == EMBER {
        // owner (signer), Ember state, USDC mint, PhUSD mint, USDC account,
        // PhUSD account, Ember vault, token program: the same order both
        // ways (wrap: USDC in, PhUSD out; unwrap: the reverse).
        if a.len() != 8 || fmt(&key(1)?) != EMBER_STATE || fmt(&key(7)?) != TOKEN || fmt(&key(2)?) != USDC || fmt(&key(3)?) != PHUSD {
            return Err("unexpected Ember accounts");
        }
        signer_me(0)?;
        if mint_of(4)? != USDC || mint_of(5)? != PHUSD {
            return Err("venue call sends the result to someone else");
        }
        return match disc {
            EMBER_DEPOSIT if d.len() == 16 => Ok(vec![(format!("spl:{USDC}"), u64_at(d, 8)? as u128)]),
            EMBER_WITHDRAW if d.len() == 9 || d.len() == 17 => Ok(vec![]),
            _ => Err("only wrapping USDC into Phoenix collateral and back"),
        };
    }
    head(ix)?;
    match disc {
        // trader (signer), the account's PhUSD account, trader account, global vault, token program, arenas.
        DEPOSIT_FUNDS => {
            signer_me(3)?;
            if mint_of(4)? != PHUSD {
                return Err("deposits come from the account's own collateral account");
            }
            mine(5)?;
            Ok(vec![])
        }
        // trader (signer), trader account, perp asset map, global vault, destination, token program, ...
        WITHDRAW_FUNDS => {
            signer_me(3)?;
            mine(4)?;
            if mint_of(7)? != PHUSD {
                return Err("venue call sends the result to someone else");
            }
            Ok(vec![])
        }
        // trader (signer), source, destination: both the account's.
        TRANSFER_COLLATERAL => {
            signer_me(3)?;
            mine(4)?;
            mine(5)?;
            Ok(vec![])
        }
        // trader wallet, child, parent; sync: trader wallet, parent, child.
        CHILD_TO_PARENT | SYNC_PARENT_TO_CHILD => {
            if key(3)? != own.me {
                return Err("not the account's own Phoenix account");
            }
            mine(4)?;
            mine(5)?;
            Ok(vec![])
        }
        // payer (signer), trader (authority), trader account, system program;
        // max_positions u32, preferences u32, pda index u8, subaccount u8.
        REGISTER_TRADER => {
            signer_me(3)?;
            // An isolated subaccount holds one position (and so its rent is known).
            if key(4)? != own.me || d.len() != 18 || d[16] != 0 || d[8..12] != [1, 0, 0, 0] {
                return Err("only the account's own trader accounts");
            }
            let sub = d[17];
            if sub == 0 || !desks.iter().any(|x| x.sub == sub) {
                return Err("a subaccount the grant doesn't list");
            }
            let t5 = key(5)?;
            if !own.traders.iter().any(|(t, s)| *s == sub && *t == t5) {
                return Err("not the account's own Phoenix account");
            }
            Ok(vec![("sol".into(), SUBACCOUNT_RENT)])
        }
        // authority (signer), trader account, new position authority.
        DELEGATE_TRADER => {
            signer_me(3)?;
            let t = key(4)?;
            let sub = own.traders.iter().find(|(x, _)| *x == t).map(|(_, s)| *s).ok_or("not the account's own Phoenix account")?;
            let to = key(5)?;
            let desk = desks.iter().find(|x| x.sub == sub).ok_or("a subaccount the grant doesn't list")?;
            if to != own.me && fmt(&to) != desk.trader_key {
                return Err("delegates only to the account's own trading key");
            }
            Ok(vec![])
        }
        PLACE_MARKET_ORDER => Err("orders go through the trading key"),
        _ => Err("not a Phoenix instruction the extension knows"),
    }
}

/// The trading key's transaction: compute budget and orders only. `trader`
/// is the subaccount's trader account (the extension derived it). Returns
/// the spends (`phoenix:<market>`, base lots).
pub fn check_order(desk: &PerpsDesk, key: &[u8; 32], trader: &[u8; 32], ixs: &[Ix]) -> Result<Vec<(String, u128)>, &'static str> {
    let mut spends: Vec<(String, u128)> = vec![];
    let mut orders = 0;
    for ix in ixs {
        if ix.accounts.iter().any(|m| m.signer && m.key != *key) {
            return Err("only the trading key may sign");
        }
        let p = fmt(&ix.program);
        if p == COMPUTE_BUDGET {
            let d = &ix.data;
            if !ix.accounts.is_empty() || !matches!((d.first(), d.len()), (Some(2), 5) | (Some(3), 9)) {
                return Err("only compute limit and price");
            }
            if d[0] == 3 && u64_at(d, 1)? > crate::sol::MAX_CU_PRICE {
                return Err("priority fee too high");
            }
            continue;
        }
        if p != PROGRAM {
            return Err("the trading key only places Phoenix orders");
        }
        head(ix)?;
        let a = &ix.accounts;
        let d = &ix.data;
        // place_market_order: trader (signer), trader account, perp asset map,
        // arenas, orderbook, spline collection; data: discriminant, the
        // immediate-or-cancel packet tag (2), side, price Option<u64>,
        // base lots u64, quote lots Option<u64>, ...
        if d.get(..8) != Some(&PLACE_MARKET_ORDER[..]) || d.get(8) != Some(&2) {
            return Err("only immediate-or-cancel orders");
        }
        if a.len() < 9 || a[3].key != *key || !a[3].signer || a[4].key != *trader || fmt(&a[5].key) != PERP_ASSET_MAP {
            return Err("an order on the subaccount the grant names, signed by its trading key");
        }
        let market = fmt(&a[a.len() - 2].key);
        let m = desk.markets.iter().find(|x| x.market == market).ok_or("not an allowed market")?;
        if d.get(10) != Some(&1) {
            return Err("an order needs a limit price");
        }
        let lots = u64_at(d, 19)?;
        if d.get(27) != Some(&0) {
            return Err("size orders in base lots");
        }
        if lots == 0 || lots > m.max_base_lots.0 {
            return Err("order larger than the grant allows");
        }
        orders += 1;
        let asset = format!("phoenix:{market}");
        match spends.iter_mut().find(|(x, _)| *x == asset) {
            Some((_, v)) => *v += lots as u128,
            None => spends.push((asset, lots as u128)),
        }
    }
    if orders == 0 {
        return Err("no order");
    }
    Ok(spends)
}

/// Whether `ix` is an SPL Token Approve of the account's own PhUSD account
/// to Ember's state, for exactly the amount a later Ember unwrap in `rest`
/// takes. Errors on an approve of PhUSD to Ember that doesn't match one.
pub fn ember_approve(own: &Own, ix: &Ix, rest: &[Ix]) -> Result<bool, &'static str> {
    // Approve (4): source, delegate, owner (signer); amount u64.
    let a = &ix.accounts;
    if fmt(&ix.program) != TOKEN || ix.data.len() != 9 || ix.data[0] != 4 || a.len() != 3 || fmt(&a[1].key) != EMBER_STATE {
        return Ok(false);
    }
    if own.mine(&a[0].key).map(|m| fmt(&m)).as_deref() != Some(PHUSD) || a[2].key != own.me || !a[2].signer {
        return Err("only the account's own collateral, approved to Ember");
    }
    let amount = u64_at(&ix.data, 1)?;
    let unwraps = |x: &Ix| fmt(&x.program) == EMBER && x.data.len() == 17 && x.data[..8] == EMBER_WITHDRAW && x.data[8] == 1 && u64_at(&x.data, 9) == Ok(amount);
    if amount == 0 || !rest.iter().any(unwraps) {
        return Err("an approval to Ember must be used up by an unwrap in the same transaction");
    }
    Ok(true)
}

pub fn sub_ok(sub: u8) -> bool {
    (1..=MAX_SUB).contains(&sub)
}
