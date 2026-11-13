//! Solana transactions for Chain Signatures (ed25519, path `solana`).
//!
//! The agent sends instructions; the extension checks every one against a
//! decoder it knows and compiles the legacy message itself, so what the MPC
//! signs is exactly what was checked. Legacy messages only: a v0 message's
//! lookup tables would name accounts the extension can't see.
//!
//! Rules:
//! - the account (the grant's `self_address`) is the fee payer and the only
//!   signer;
//! - every instruction's program must have a decoder here; any other program
//!   is refused;
//! - a protocol instruction must name a venue the grant lists (the program
//!   and its pool or market) and must send its result to the account's own
//!   token account (an associated token account derived here) or the
//!   account itself;
//! - SOL and token payments go only to listed recipients;
//! - what the account pays or puts into a protocol counts against the caps:
//!   `sol` (in lamports, wrapped SOL included, fees too) and `spl:<mint>`.
//!   Withdrawals back to the account itself don't count.

use near_sdk::env;

use crate::rules::{SolGrant, SolIx};

pub const SYSTEM: &str = "11111111111111111111111111111111";
pub const COMPUTE_BUDGET: &str = "ComputeBudget111111111111111111111111111111";
pub const TOKEN: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
pub const TOKEN_2022: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
pub const ATA_PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
pub const WSOL: &str = "So11111111111111111111111111111111111111112";
/// SPL stake pool programs: Jito's on mainnet (from Jito's deployed-programs
/// page) and Jito's current devnet deployment (checked on devnet: an
/// upgradeable program owning the JitoSOL pool).
pub const STAKE_POOL_PROGRAMS: [&str; 2] = ["SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy", "DPoo15wWDqpPJJtS2MUZ49aRxqz5ZaaJCJP4z8bLuib"];
/// Kamino Lend (klend), the same ID on mainnet and devnet (checked on devnet).
pub const KAMINO: &str = "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD";
/// Anchor discriminators, sha256("global:<name>")[..8], matching
/// @kamino-finance/klend-sdk 13.0.3's generated instructions.
const K_REFRESH_RESERVE: [u8; 8] = [2, 218, 138, 235, 79, 201, 25, 102];
const K_DEPOSIT: [u8; 8] = [169, 201, 30, 126, 6, 205, 102, 68];
const K_REDEEM: [u8; 8] = [234, 117, 181, 125, 185, 142, 220, 29];

/// Rent for a token account (165 bytes), counted when the agent creates one.
pub const TOKEN_ACCOUNT_RENT: u128 = 2_039_280;
/// The most a compute-unit price may be, in micro-lamports per unit.
pub const MAX_CU_PRICE: u64 = 1_000_000;
/// Solana's compute limit when a transaction doesn't set one.
const DEFAULT_CU: u64 = 1_400_000;
/// A signed legacy transaction must fit in 1,232 bytes: one signature and
/// its count take 65.
pub const MAX_MESSAGE: usize = 1232 - 65;

pub fn b58(s: &str) -> Result<[u8; 32], &'static str> {
    bs58::decode(s).into_vec().ok().and_then(|v| v.try_into().ok()).ok_or("not a Solana address")
}

pub fn fmt(k: &[u8; 32]) -> String {
    bs58::encode(k).into_string()
}

/// An associated token account, from its seeds and the bump the agent
/// supplies. A wrong bump gives an address no token account can live at
/// (only the ATA program, with the canonical bump, can create one there), so
/// the transaction would fail, never pay someone else.
pub fn ata(owner: &[u8; 32], mint: &[u8; 32], token_program: &[u8; 32], bump: u8) -> [u8; 32] {
    let mut b = Vec::with_capacity(32 * 4 + 1 + 21);
    b.extend_from_slice(owner);
    b.extend_from_slice(token_program);
    b.extend_from_slice(mint);
    b.push(bump);
    b.extend_from_slice(&b58(ATA_PROGRAM).unwrap());
    b.extend_from_slice(b"ProgramDerivedAddress");
    env::sha256_array(&b)
}

pub struct Meta {
    pub key: [u8; 32],
    pub signer: bool,
    pub writable: bool,
}

pub struct Ix {
    pub program: [u8; 32],
    pub accounts: Vec<Meta>,
    pub data: Vec<u8>,
}

/// The account's token accounts the agent says it uses: address → mint.
pub struct Own {
    pub me: [u8; 32],
    /// (address, owner, mint)
    pub atas: Vec<([u8; 32], [u8; 32], [u8; 32])>,
}

impl Own {
    /// The mint of the account's own token account at `addr`, if it is one.
    fn mine(&self, addr: &[u8; 32]) -> Option<[u8; 32]> {
        self.atas.iter().find(|(a, o, _)| a == addr && o == &self.me).map(|(_, _, m)| *m)
    }
    fn owner_of(&self, addr: &[u8; 32]) -> Option<([u8; 32], [u8; 32])> {
        self.atas.iter().find(|(a, _, _)| a == addr).map(|(_, o, m)| (*o, *m))
    }
}

fn asset(mint: &[u8; 32]) -> String {
    let m = fmt(mint);
    if m == WSOL { "sol".into() } else { format!("spl:{m}") }
}

fn u64_at(d: &[u8], at: usize) -> Result<u64, &'static str> {
    d.get(at..at + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap())).ok_or("instruction data too short")
}

fn venue_ok(g: &SolGrant, program: &str, scope: &[u8; 32], queued: bool) -> Result<(), &'static str> {
    let s = fmt(scope);
    if queued || g.venues.iter().any(|v| v.program == program && v.scope == s) {
        Ok(())
    } else {
        Err("not an allowed venue")
    }
}

/// Checks one instruction and returns what it spends.
fn check_ix(g: &SolGrant, own: &Own, ix: &Ix, queued: bool, cu: &mut (Option<u64>, u64)) -> Result<Vec<(String, u128)>, &'static str> {
    // The account is the only signer, anywhere.
    if ix.accounts.iter().any(|m| m.signer && m.key != own.me) {
        return Err("only the account may sign");
    }
    let program = fmt(&ix.program);
    let a = &ix.accounts;
    let d = &ix.data;
    let key = |i: usize| a.get(i).map(|m| m.key).ok_or("missing account");
    let n = |want: usize| if a.len() == want { Ok(()) } else { Err("wrong number of accounts") };
    let signer_me = |i: usize| match a.get(i) {
        Some(m) if m.key == own.me && m.signer => Ok(()),
        _ => Err("the account must sign this"),
    };
    let p = program.as_str();
    if p == COMPUTE_BUDGET {
        match (d.first(), d.len()) {
            (Some(2), 5) => cu.0 = Some(u32::from_le_bytes(d[1..5].try_into().unwrap()) as u64),
            (Some(3), 9) => {
                let price = u64_at(d, 1)?;
                if price > MAX_CU_PRICE {
                    return Err("priority fee too high");
                }
                cu.1 = price;
            }
            _ => return Err("only compute limit and price"),
        }
        n(0)?;
        return Ok(vec![]);
    }
    if p == SYSTEM {
        // Transfer only.
        if d.len() != 12 || d[..4] != [2, 0, 0, 0] {
            return Err("only SOL transfers on the System program");
        }
        n(2)?;
        signer_me(0)?;
        let to = key(1)?;
        let lamports = u64_at(d, 4)? as u128;
        // Wrapping SOL into the account's own wrapped-SOL account isn't a payment.
        if own.mine(&to).is_some_and(|m| fmt(&m) == WSOL) {
            return Ok(vec![]);
        }
        if to == own.me {
            return Err("transfer to itself");
        }
        if !queued && !g.recipients.contains(&fmt(&to)) {
            return Err("recipient not allowed");
        }
        return Ok(vec![("sol".into(), lamports)]);
    }
    if p == ATA_PROGRAM {
        // Create or CreateIdempotent: payer, ata, wallet, mint, system, token program.
        if !(d.is_empty() || d == &[0] || d == &[1]) {
            return Err("only creating a token account");
        }
        n(6)?;
        signer_me(0)?;
        let (ata_addr, wallet, mint, tp) = (key(1)?, key(2)?, key(3)?, key(5)?);
        if fmt(&tp) != TOKEN && fmt(&tp) != TOKEN_2022 {
            return Err("unknown token program");
        }
        match own.owner_of(&ata_addr) {
            Some((o, m)) if o == wallet && m == mint => {}
            _ => return Err("token account doesn't match its owner and mint"),
        }
        if wallet != own.me && !queued && !g.recipients.contains(&fmt(&wallet)) {
            return Err("recipient not allowed");
        }
        return Ok(vec![("sol".into(), TOKEN_ACCOUNT_RENT)]);
    }
    if p == TOKEN || p == TOKEN_2022 {
        match d.first() {
            // TransferChecked: source, mint, destination, authority.
            Some(12) if d.len() == 10 => {
                n(4)?;
                signer_me(3)?;
                let (src, mint, dst) = (key(0)?, key(1)?, key(2)?);
                if own.mine(&src) != Some(mint) {
                    return Err("tokens must come from the account's own token account");
                }
                let (to, dmint) = own.owner_of(&dst).ok_or("unknown destination token account")?;
                if dmint != mint || to == own.me {
                    return Err("bad destination");
                }
                if !queued && !g.recipients.contains(&fmt(&to)) {
                    return Err("recipient not allowed");
                }
                Ok(vec![(asset(&mint), u64_at(d, 1)? as u128)])
            }
            // SyncNative on the account's wrapped-SOL account.
            Some(17) if d.len() == 1 => {
                n(1)?;
                if own.mine(&key(0)?).map(|m| fmt(&m)) != Some(WSOL.into()) {
                    return Err("sync only the account's wrapped SOL");
                }
                Ok(vec![])
            }
            // CloseAccount: account, destination, owner. Rent comes back.
            Some(9) if d.len() == 1 => {
                n(3)?;
                signer_me(2)?;
                if own.mine(&key(0)?).is_none() || key(1)? != own.me {
                    return Err("close only the account's own token account, to itself");
                }
                Ok(vec![])
            }
            _ => Err("only transfers, wrapping and closing on the token program"),
        }
    } else if STAKE_POOL_PROGRAMS.contains(&p) {
        match d.first() {
            // DepositSol: pool, withdraw authority, reserve, from (signer),
            // pool tokens to, manager fee, referrer, mint, system, token.
            Some(14) if d.len() == 9 => {
                n(10)?;
                venue_ok(g, p, &key(0)?, queued)?;
                signer_me(3)?;
                let to = key(4)?;
                if own.mine(&to) != Some(key(7)?) {
                    return Err("venue call sends the result to someone else");
                }
                // The referrer gets part of the fee: it must be the account too.
                if key(6)? != to {
                    return Err("referral fee must go to the account");
                }
                if fmt(&key(8)?) != SYSTEM || fmt(&key(9)?) != TOKEN {
                    return Err("unexpected program");
                }
                Ok(vec![("sol".into(), u64_at(d, 1)? as u128)])
            }
            // WithdrawSol: pool, withdraw authority, owner (signer), pool
            // tokens from, reserve, lamports to, manager fee, mint, clock,
            // stake history, stake program, token.
            Some(16) if d.len() == 9 => {
                n(12)?;
                venue_ok(g, p, &key(0)?, queued)?;
                signer_me(2)?;
                let mint = key(7)?;
                if own.mine(&key(3)?) != Some(mint) {
                    return Err("pool tokens must come from the account's own token account");
                }
                if key(5)? != own.me {
                    return Err("venue call sends the result to someone else");
                }
                if fmt(&key(11)?) != TOKEN {
                    return Err("unexpected program");
                }
                u64_at(d, 1)?;
                // Back to the account itself: not a spend (as Aave withdrawals).
                Ok(vec![])
            }
            _ => Err("only SOL deposits and withdrawals on a stake pool"),
        }
    } else if p == KAMINO {
        let disc: [u8; 8] = d.get(..8).and_then(|b| b.try_into().ok()).ok_or("instruction data too short")?;
        match disc {
            // refresh_reserve: reserve, market, 4 oracle slots.
            K_REFRESH_RESERVE if d.len() == 8 => {
                n(6)?;
                venue_ok(g, p, &key(1)?, queued)?;
                Ok(vec![])
            }
            // deposit_reserve_liquidity: owner, reserve, market, market
            // authority, liquidity mint, supply, collateral mint, source,
            // destination collateral, collateral token program, liquidity
            // token program, instructions sysvar.
            K_DEPOSIT if d.len() == 16 => {
                n(12)?;
                signer_me(0)?;
                venue_ok(g, p, &key(2)?, queued)?;
                let (mint, cmint) = (key(4)?, key(6)?);
                if own.mine(&key(7)?) != Some(mint) {
                    return Err("tokens must come from the account's own token account");
                }
                if own.mine(&key(8)?) != Some(cmint) {
                    return Err("venue call sends the result to someone else");
                }
                Ok(vec![(asset(&mint), u64_at(d, 8)? as u128)])
            }
            // redeem_reserve_collateral: owner, market, reserve, market
            // authority, liquidity mint, collateral mint, supply, source
            // collateral, destination liquidity, collateral token program,
            // liquidity token program, instructions sysvar.
            K_REDEEM if d.len() == 16 => {
                n(12)?;
                signer_me(0)?;
                venue_ok(g, p, &key(1)?, queued)?;
                let (mint, cmint) = (key(4)?, key(5)?);
                if own.mine(&key(7)?) != Some(cmint) {
                    return Err("tokens must come from the account's own token account");
                }
                if own.mine(&key(8)?) != Some(mint) {
                    return Err("venue call sends the result to someone else");
                }
                u64_at(d, 8)?;
                // Back to the account itself: not a spend.
                Ok(vec![])
            }
            _ => Err("only refresh, deposit and redeem on Kamino"),
        }
    } else {
        Err("no decoder for this Solana program")
    }
}

/// Parses the agent's instructions.
pub fn parse(ixs: &[SolIx]) -> Result<Vec<Ix>, &'static str> {
    if ixs.is_empty() || ixs.len() > 12 {
        return Err("1 to 12 instructions");
    }
    ixs.iter()
        .map(|i| {
            Ok(Ix {
                program: b58(&i.program)?,
                accounts: i.accounts.iter().map(|m| Ok(Meta { key: b58(&m.pubkey)?, signer: m.signer, writable: m.writable })).collect::<Result<_, &'static str>>()?,
                data: hex::decode(&i.data).map_err(|_| "instruction data must be hex")?,
            })
        })
        .collect()
}

/// Checks every instruction and compiles the message. Returns the message
/// bytes (what the account's key signs) and the spends.
pub fn check(g: &SolGrant, own: &Own, ixs: &[Ix], blockhash: &[u8; 32], queued: bool) -> Result<(Vec<u8>, Vec<(String, u128)>), &'static str> {
    let mut spends: Vec<(String, u128)> = vec![];
    let mut cu = (None, 0u64);
    for ix in ixs {
        for (asset, amount) in check_ix(g, own, ix, queued, &mut cu)? {
            match spends.iter_mut().find(|(a, _)| *a == asset) {
                Some((_, v)) => *v = v.checked_add(amount).ok_or("overflow")?,
                None => spends.push((asset, amount)),
            }
        }
    }
    // The priority fee: price × limit, rounded up; plus 5,000 for the signature.
    let limit = cu.0.unwrap_or(DEFAULT_CU);
    let fee = 5_000u128 + ((cu.1 as u128) * (limit as u128)).div_ceil(1_000_000);
    match spends.iter_mut().find(|(a, _)| a == "sol") {
        Some((_, v)) => *v += fee,
        None => spends.push(("sol".into(), fee)),
    }
    let msg = compile(&own.me, ixs, blockhash)?;
    if msg.len() > MAX_MESSAGE {
        return Err("transaction too large");
    }
    Ok((msg, spends))
}

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

/// Compiles a legacy message with `payer` as fee payer and only signer.
pub fn compile(payer: &[u8; 32], ixs: &[Ix], blockhash: &[u8; 32]) -> Result<Vec<u8>, &'static str> {
    // Each key once, with its strongest flags, in first-seen order.
    let mut keys: Vec<([u8; 32], bool, bool)> = vec![(*payer, true, true)];
    let mut add = |k: [u8; 32], s: bool, w: bool| match keys.iter_mut().find(|x| x.0 == k) {
        Some(x) => {
            x.1 |= s;
            x.2 |= w;
        }
        None => keys.push((k, s, w)),
    };
    for ix in ixs {
        for m in &ix.accounts {
            add(m.key, m.signer, m.writable);
        }
    }
    for ix in ixs {
        add(ix.program, false, false);
    }
    if keys.iter().any(|k| k.1 && k.0 != *payer) {
        return Err("only the account may sign");
    }
    if keys.len() > 64 {
        return Err("too many accounts");
    }
    // Payer first; then writable non-signers, then read-only non-signers.
    let mut ordered = vec![keys[0]];
    ordered.extend(keys[1..].iter().filter(|k| k.2));
    ordered.extend(keys[1..].iter().filter(|k| !k.2));
    let readonly_unsigned = ordered.iter().filter(|k| !k.2).count();
    let idx = |k: &[u8; 32]| ordered.iter().position(|x| x.0 == *k).unwrap() as u8;
    let mut m = vec![1u8, 0, readonly_unsigned as u8];
    compact_u16(&mut m, ordered.len());
    for k in &ordered {
        m.extend_from_slice(&k.0);
    }
    m.extend_from_slice(blockhash);
    compact_u16(&mut m, ixs.len());
    for ix in ixs {
        m.push(idx(&ix.program));
        compact_u16(&mut m, ix.accounts.len());
        for a in &ix.accounts {
            m.push(idx(&a.key));
        }
        compact_u16(&mut m, ix.data.len());
        m.extend_from_slice(&ix.data);
    }
    Ok(m)
}
