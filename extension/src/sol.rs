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
//!   `sol` (in lamports, wrapped SOL included, fees and rent too) and
//!   `spl:<mint>`. Withdrawals back to the account itself don't count.
//! - Kamino borrows count against `borrow:solana:<mint>`, and every
//!   obligation instruction must name the account's own obligation (derived
//!   here from the account and the market), so a repay can't pay someone
//!   else's loan.

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
const K_INIT_USER_METADATA: [u8; 8] = [117, 169, 176, 69, 197, 23, 15, 162];
const K_INIT_OBLIGATION: [u8; 8] = [251, 10, 231, 76, 27, 11, 159, 96];
const K_REFRESH_OBLIGATION: [u8; 8] = [33, 132, 147, 228, 151, 192, 72, 89];
const K_DEPOSIT_COLLATERAL: [u8; 8] = [129, 199, 4, 2, 222, 39, 26, 46];
const K_BORROW: [u8; 8] = [121, 127, 18, 204, 73, 245, 225, 65];
const K_REPAY: [u8; 8] = [145, 178, 13, 225, 76, 240, 147, 72];
const K_WITHDRAW_COLLATERAL: [u8; 8] = [75, 93, 93, 220, 34, 150, 218, 196];
/// Rent for Kamino's accounts, counted when the agent creates them:
/// (size + 128) × 6,960 lamports, sizes from klend-sdk 13.0.3's layouts
/// (UserMetadata 1,032 bytes, Obligation 3,344, discriminators included).
pub const KAMINO_USER_METADATA_RENT: u128 = (1032 + 128) * 6960;
pub const KAMINO_OBLIGATION_RENT: u128 = (3344 + 128) * 6960;
/// Tensor's marketplace (TCMP), the same ID on mainnet and devnet
/// (@tensor-foundation/marketplace 1.0.0, as protean-bot pins it).
pub const TENSOR: &str = "TCMPhJdwDryooaGtiocG1u3xcYbRpiJzb283XfCZsDp";
/// Jupiter v6 (mainnet; on a Surfpool fork for tests). Discriminators are
/// Anchor's sha256("global:<name>")[..8] for the names in Jupiter's IDL
/// (jup-ag/jupiter-cpi idl.json), and match its API's instructions.
pub const JUPITER: &str = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";
const J_ROUTE: [u8; 8] = [0xe5, 0x17, 0xcb, 0x97, 0x7a, 0xe3, 0xad, 0x2a];
const J_SHARED_ROUTE: [u8; 8] = [0xc1, 0x20, 0x9b, 0x33, 0x41, 0xd6, 0x9c, 0x81];
/// Phoenix v1 (mainnet; on a Surfpool fork for tests): Ellipsis-Labs/phoenix-v1.
pub const PHOENIX: &str = "PhoeNiXZ8ByJGLkxNfZRnkUfjvmuYqLR89jjFHGqdXY";
/// Phoenix markets with their lot sizes (fixed when a market is made), from
/// each market's header on mainnet (13 November 2026): market, base mint,
/// base lot size, quote mint, quote lot size.
const PHOENIX_MARKETS: [(&str, &str, u64, &str, u64); 1] = [
    ("4DoNfFBfF7UokCC2FQzriy7yHK6DY6NVdYpuekQ5pRgg", WSOL, 1_000_000, "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v", 1),
];
/// The most slippage an agent's Jupiter swap may allow.
pub const JUP_MAX_SLIPPAGE_BPS: u16 = 300;
/// That client's discriminators.
const T_LIST_LEGACY: [u8; 8] = [6, 110, 255, 18, 16, 36, 8, 30];
const T_BUY_LEGACY: [u8; 8] = [68, 127, 43, 8, 212, 31, 249, 114];
const T_DELIST_LEGACY: [u8; 8] = [88, 35, 231, 184, 110, 218, 149, 23];
/// Rent for a Tensor listing account, counted when the agent lists: as
/// protean-bot budgets it, rent-exempt for 400 bytes.
pub const TENSOR_LIST_RENT: u128 = (400 + 128) * 6960;
/// Tensor's taker fee and a maker broker's share, counted on a buy (3%).
pub const TENSOR_FEE_PCT: u128 = 3;
pub const RENT_SYSVAR: &str = "SysvarRent111111111111111111111111111111111";
pub const INSTRUCTIONS_SYSVAR: &str = "Sysvar1nstructions1111111111111111111111111";

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

/// A program-derived address from its seeds and bump (no curve check: a
/// wrong bump gives an address the program never creates an account at).
pub fn pda(seeds: &[&[u8]], bump: u8, program: &[u8; 32]) -> [u8; 32] {
    let mut b = Vec::new();
    for s in seeds {
        b.extend_from_slice(s);
    }
    b.push(bump);
    b.extend_from_slice(program);
    b.extend_from_slice(b"ProgramDerivedAddress");
    env::sha256_array(&b)
}

/// The account's default Kamino obligation in `market` (tag 0, id 0, seed
/// accounts the default key; klend-sdk getObligationPdaWithArgs).
pub fn kamino_obligation(me: &[u8; 32], market: &[u8; 32], bump: u8) -> [u8; 32] {
    pda(&[&[0], &[0], me, market, &[0; 32], &[0; 32]], bump, &b58(KAMINO).unwrap())
}

/// The account's token accounts and obligations the agent says it uses.
pub struct Own {
    pub me: [u8; 32],
    /// (address, owner, mint)
    pub atas: Vec<([u8; 32], [u8; 32], [u8; 32])>,
    /// The account's own Kamino obligations.
    pub obligations: Vec<[u8; 32]>,
    /// The account's own Phoenix perps trader accounts, with their subaccount.
    pub traders: Vec<([u8; 32], u8)>,
}

impl Own {
    /// The mint of the account's own token account at `addr`, if it is one.
    pub fn mine(&self, addr: &[u8; 32]) -> Option<[u8; 32]> {
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
        // Fees for the account's own Phoenix trading keys.
        if !queued && !g.recipients.contains(&fmt(&to)) && !g.perps.iter().any(|x| x.trader_key == fmt(&to)) {
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
        let obligation = |i: usize| -> Result<(), &'static str> {
            if own.obligations.contains(&key(i)?) { Ok(()) } else { Err("not the account's own obligation") }
        };
        let token_programs = |at: &[usize]| -> Result<(), &'static str> {
            for &i in at {
                let t = fmt(&key(i)?);
                if t != TOKEN && t != TOKEN_2022 {
                    return Err("unknown token program");
                }
            }
            if fmt(&key(at[at.len() - 1] + 1)?) != INSTRUCTIONS_SYSVAR {
                return Err("unexpected account");
            }
            Ok(())
        };
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
            // init_user_metadata(user_lookup_table): owner, fee payer,
            // user metadata, referrer (none: the program), rent, system.
            // The address is the program's to check (seeds "user_meta" and
            // the owner); nothing is paid but its rent.
            K_INIT_USER_METADATA if d.len() == 40 => {
                n(6)?;
                signer_me(0)?;
                signer_me(1)?;
                if !queued && !g.venues.iter().any(|v| v.program == KAMINO) {
                    return Err("not an allowed venue");
                }
                if d[8..] != [0u8; 32] {
                    return Err("no lookup table");
                }
                if key(3)? != ix.program || fmt(&key(4)?) != RENT_SYSVAR || fmt(&key(5)?) != SYSTEM {
                    return Err("unexpected account");
                }
                Ok(vec![("sol".into(), KAMINO_USER_METADATA_RENT)])
            }
            // init_obligation(tag 0, id 0): owner, fee payer, obligation,
            // market, seed 1, seed 2 (default key), user metadata, rent, system.
            K_INIT_OBLIGATION if d.len() == 10 => {
                n(9)?;
                signer_me(0)?;
                signer_me(1)?;
                venue_ok(g, p, &key(3)?, queued)?;
                if d[8..] != [0, 0] || key(4)? != [0; 32] || key(5)? != [0; 32] {
                    return Err("only the default obligation");
                }
                obligation(2)?;
                if fmt(&key(7)?) != RENT_SYSVAR || fmt(&key(8)?) != SYSTEM {
                    return Err("unexpected account");
                }
                Ok(vec![("sol".into(), KAMINO_OBLIGATION_RENT)])
            }
            // refresh_obligation: market, obligation, then its reserves.
            K_REFRESH_OBLIGATION if d.len() == 8 => {
                if a.len() < 2 || a.len() > 2 + 16 {
                    return Err("wrong number of accounts");
                }
                venue_ok(g, p, &key(0)?, queued)?;
                obligation(1)?;
                Ok(vec![])
            }
            // deposit_reserve_liquidity_and_obligation_collateral: owner,
            // obligation, market, market authority, reserve, liquidity mint,
            // supply, collateral mint, collateral vault, source, placeholder
            // (the program), collateral and liquidity token programs, sysvar.
            K_DEPOSIT_COLLATERAL if d.len() == 16 => {
                n(14)?;
                signer_me(0)?;
                obligation(1)?;
                venue_ok(g, p, &key(2)?, queued)?;
                let mint = key(5)?;
                if own.mine(&key(9)?) != Some(mint) {
                    return Err("tokens must come from the account's own token account");
                }
                if key(10)? != ix.program {
                    return Err("unexpected account");
                }
                token_programs(&[11, 12])?;
                Ok(vec![(asset(&mint), u64_at(d, 8)? as u128)])
            }
            // borrow_obligation_liquidity: owner, obligation, market, market
            // authority, reserve, liquidity mint, supply, fee receiver,
            // destination, referrer token state (none: the program), token
            // program, sysvar. What is borrowed counts against
            // `borrow:solana:<mint>`; it goes only to the account.
            K_BORROW if d.len() == 16 => {
                n(12)?;
                signer_me(0)?;
                obligation(1)?;
                venue_ok(g, p, &key(2)?, queued)?;
                let mint = key(5)?;
                if own.mine(&key(8)?) != Some(mint) {
                    return Err("venue call sends the result to someone else");
                }
                if key(9)? != ix.program {
                    return Err("no referrer");
                }
                token_programs(&[10])?;
                Ok(vec![(format!("borrow:solana:{}", fmt(&mint)), u64_at(d, 8)? as u128)])
            }
            // repay_obligation_liquidity: owner, obligation (the account's
            // own: klend lets anyone repay any obligation), market, reserve,
            // liquidity mint, supply, source, token program, sysvar.
            K_REPAY if d.len() == 16 => {
                n(9)?;
                signer_me(0)?;
                obligation(1)?;
                venue_ok(g, p, &key(2)?, queued)?;
                let mint = key(4)?;
                if own.mine(&key(6)?) != Some(mint) {
                    return Err("tokens must come from the account's own token account");
                }
                token_programs(&[7])?;
                Ok(vec![(asset(&mint), u64_at(d, 8)? as u128)])
            }
            // withdraw_obligation_collateral_and_redeem_reserve_collateral:
            // owner, obligation, market, market authority, reserve,
            // liquidity mint, collateral vault, collateral mint, supply,
            // destination, placeholder (the program), collateral and
            // liquidity token programs, sysvar. Back to the account: not a
            // spend (klend refuses one that leaves the loan unhealthy).
            K_WITHDRAW_COLLATERAL if d.len() == 16 => {
                n(14)?;
                signer_me(0)?;
                obligation(1)?;
                venue_ok(g, p, &key(2)?, queued)?;
                if own.mine(&key(9)?) != Some(key(5)?) {
                    return Err("venue call sends the result to someone else");
                }
                if key(10)? != ix.program {
                    return Err("unexpected account");
                }
                token_programs(&[11, 12])?;
                u64_at(d, 8)?;
                Ok(vec![])
            }
            _ => Err("not a Kamino instruction the extension knows"),
        }
    } else if p == JUPITER {
        let disc: [u8; 8] = d.get(..8).and_then(|b| b.try_into().ok()).ok_or("instruction data too short")?;
        if !queued && !g.venues.iter().any(|v| v.program == JUPITER && v.scope == JUPITER) {
            return Err("not an allowed venue");
        }
        // The route plan varies in length; the fixed arguments end the data:
        // in_amount u64, quoted_out_amount u64, slippage_bps u16, platform_fee_bps u8.
        if d.len() < 8 + 4 + 19 {
            return Err("instruction data too short");
        }
        let t = d.len() - 19;
        let in_amount = u64_at(d, t)? as u128;
        let slippage = u16::from_le_bytes([d[t + 16], d[t + 17]]);
        if d[t + 18] != 0 {
            return Err("no platform fee");
        }
        if slippage > JUP_MAX_SLIPPAGE_BPS {
            return Err("slippage over 3%");
        }
        // An optional account left out is Jupiter's own program ID.
        let none = |i: usize| -> Result<bool, &'static str> { Ok(key(i)? == ix.program) };
        match disc {
            // route: token program, user (signer), user source, user
            // destination, destination (optional), destination mint,
            // platform fee account (optional), event authority, program;
            // then the route's accounts.
            J_ROUTE => {
                if a.len() < 9 {
                    return Err("wrong number of accounts");
                }
                signer_me(1)?;
                let src = own.mine(&key(2)?).ok_or("the source must be the account's own token account")?;
                let mint = key(5)?;
                if own.mine(&key(3)?) != Some(mint) || !(none(4)? || own.mine(&key(4)?) == Some(mint)) {
                    return Err("venue call sends the result to someone else");
                }
                if !none(6)? {
                    return Err("no platform fee");
                }
                Ok(vec![(asset(&src), in_amount)])
            }
            // shared_accounts_route: token program, program authority, user
            // (signer), source, program source, program destination,
            // destination, source mint, destination mint, platform fee
            // account (optional), token-2022, event authority, program.
            J_SHARED_ROUTE => {
                if a.len() < 13 {
                    return Err("wrong number of accounts");
                }
                signer_me(2)?;
                let src = key(7)?;
                if own.mine(&key(3)?) != Some(src) {
                    return Err("the source must be the account's own token account");
                }
                if own.mine(&key(6)?) != Some(key(8)?) {
                    return Err("venue call sends the result to someone else");
                }
                if !none(9)? {
                    return Err("no platform fee");
                }
                Ok(vec![(asset(&src), in_amount)])
            }
            _ => Err("only Jupiter's route and shared-accounts route"),
        }
    } else if p == crate::perps::PROGRAM || p == crate::perps::EMBER {
        if !queued && g.perps.is_empty() {
            return Err("not an allowed venue");
        }
        crate::perps::check_authority(&g.perps, own, ix)
    } else if p == PHOENIX {
        // Swap (0) with an ImmediateOrCancel packet (2): phoenix program,
        // log authority, market, trader (signer), base account, quote
        // account, base vault, quote vault, token program.
        if d.first() != Some(&0) || d.get(1) != Some(&2) {
            return Err("only Phoenix swaps (immediate or cancel)");
        }
        n(9)?;
        if key(0)? != ix.program || fmt(&key(8)?) != TOKEN {
            return Err("unexpected program");
        }
        let market = key(2)?;
        venue_ok(g, PHOENIX, &market, queued)?;
        let &(_, base, base_lot, quote, quote_lot) = PHOENIX_MARKETS.iter().find(|m| m.0 == fmt(&market)).ok_or("unknown Phoenix market")?;
        signer_me(3)?;
        // Both sides settle into the account's own token accounts.
        if own.mine(&key(4)?).map(|m| fmt(&m)) != Some(base.to_string()) || own.mine(&key(5)?).map(|m| fmt(&m)) != Some(quote.to_string()) {
            return Err("venue call sends the result to someone else");
        }
        let side = *d.get(2).ok_or("instruction data too short")?;
        // price_in_ticks: Option<u64>.
        let at = match d.get(3) {
            Some(0) => 4,
            Some(1) => 12,
            _ => return Err("unexpected Phoenix order"),
        };
        let (base_lots, quote_lots) = (u64_at(d, at)? as u128, u64_at(d, at + 8)? as u128);
        // Phoenix takes exactly one budget; the extension asks for the one
        // that bounds what leaves the account: quote lots to buy, base lots
        // to sell.
        match side {
            0 if base_lots == 0 && quote_lots > 0 => Ok(vec![(if quote == WSOL { "sol".into() } else { format!("spl:{quote}") }, quote_lots * quote_lot as u128)]),
            1 if quote_lots == 0 && base_lots > 0 => Ok(vec![(if base == WSOL { "sol".into() } else { format!("spl:{base}") }, base_lots * base_lot as u128)]),
            0 | 1 => Err("buy in quote lots, sell in base lots"),
            _ => Err("unexpected Phoenix order"),
        }
    } else if p == TENSOR {
        let disc: [u8; 8] = d.get(..8).and_then(|b| b.try_into().ok()).ok_or("instruction data too short")?;
        let me_or_none = |i: usize| -> Result<(), &'static str> {
            let k = key(i)?;
            if k == own.me || k == ix.program { Ok(()) } else { Err("a broker that isn't the account") }
        };
        if !queued && !g.venues.iter().any(|v| v.program == TENSOR && v.scope == TENSOR) {
            return Err("not an allowed venue");
        }
        // Metaplex legacy NFTs, so no authorization data (programmable NFTs' rule sets).
        match disc {
            // buy_legacy(max_amount, optional_royalty_pct: Option<u16>, None):
            // fee vault, buyer, buyer's token account, listing's token
            // account, listing, mint, seller (paid), payer, taker broker,
            // maker broker, rent destination, token, ATA, marketplace and
            // system programs, metadata, edition, two token records, rules,
            // rules program, metadata program, sysvar, cosigner; then the
            // creators (royalties).
            T_BUY_LEGACY => {
                if a.len() < 24 {
                    return Err("wrong number of accounts");
                }
                let max = u64_at(d, 8)? as u128;
                let pct: u128 = match &d[16..] {
                    [0, 0] => 0,
                    [1, lo, hi, 0] => u16::from_le_bytes([*lo, *hi]) as u128,
                    _ => return Err("unexpected Tensor buy data"),
                };
                if pct > 100 {
                    return Err("royalty share over 100%");
                }
                if key(1)? != own.me {
                    return Err("venue call sends the result to someone else");
                }
                signer_me(7)?;
                let mint = key(5)?;
                if own.mine(&key(2)?) != Some(mint) {
                    return Err("venue call sends the result to someone else");
                }
                me_or_none(8)?;
                if key(23)? != ix.program {
                    return Err("a listing that needs Tensor's cosigner");
                }
                // The price, Tensor's fees and, at worst, royalties of the
                // share the agent opts into on top; and the new token account.
                let spend = max + max * TENSOR_FEE_PCT / 100 + max * pct / 100 + TOKEN_ACCOUNT_RENT;
                Ok(vec![("sol".into(), spend)])
            }
            // list_legacy(amount, expire_in_sec, currency, private_taker,
            // maker_broker, authorization_data): owner, owner's token
            // account, listing, listing's token account, mint, payer, …
            T_LIST_LEGACY => {
                n(19)?;
                signer_me(0)?;
                signer_me(5)?;
                let mint = key(4)?;
                if own.mine(&key(1)?) != Some(mint) {
                    return Err("tokens must come from the account's own token account");
                }
                let price = u64_at(d, 8)? as u128;
                let mut i = 16;
                let mut opt = |len: usize| -> Result<Option<Vec<u8>>, &'static str> {
                    match d.get(i) {
                        Some(0) => { i += 1; Ok(None) }
                        Some(1) => { let v = d.get(i + 1..i + 1 + len).ok_or("unexpected Tensor list data")?.to_vec(); i += 1 + len; Ok(Some(v)) }
                        _ => Err("unexpected Tensor list data"),
                    }
                };
                opt(8)?; // expiry: any
                if opt(32)?.is_some() {
                    return Err("only SOL listings");
                }
                if opt(32)?.is_some() {
                    return Err("no private buyer");
                }
                if let Some(b) = opt(32)? {
                    if b[..] != own.me {
                        return Err("a broker that isn't the account");
                    }
                }
                if opt(0)?.is_some() || i != d.len() {
                    return Err("unexpected Tensor list data");
                }
                if g.nft_floor.0 == 0 || price < g.nft_floor.0 {
                    return Err("listing price below the floor");
                }
                Ok(vec![("nft:solana".into(), 1), ("sol".into(), TENSOR_LIST_RENT)])
            }
            // delist_legacy(None): owner, owner's token account (the NFT
            // comes back here), listing, listing's token account, mint, …
            T_DELIST_LEGACY => {
                n(19)?;
                if d.len() != 9 || d[8] != 0 {
                    return Err("unexpected Tensor delist data");
                }
                signer_me(0)?;
                signer_me(6)?;
                if own.mine(&key(1)?) != Some(key(4)?) {
                    return Err("venue call sends the result to someone else");
                }
                Ok(vec![])
            }
            _ => Err("only buy, list and delist on Tensor"),
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
    for (i, ix) in ixs.iter().enumerate() {
        // Phoenix's withdraw flow approves Ember's state to pull the
        // collateral it unwraps: allowed only when a later instruction of
        // the same transaction unwraps exactly that amount, so no allowance
        // is left behind.
        if crate::perps::ember_approve(own, ix, &ixs[i + 1..])? {
            continue;
        }
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
