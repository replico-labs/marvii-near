//! Spending limits, so the hosted relayer can't be drained. It pays gas for
//! everyone and gives each new wallet a little NEAR; without limits, anyone
//! could create passkeys (free) and collect the gift, or send junk that
//! costs gas before the chain refuses it.
//!
//! - Paid transactions: at most `MARVII_LIMIT_WALLET_DAY` (default 50) per
//!   wallet per UTC day, and `MARVII_LIMIT_HOUR` (default 300) in total per
//!   hour.
//! - New-wallet NEAR: at most `MARVII_FUND_DAY` (default 20) wallets a day,
//!   one per IP address a day, never to a wallet that already holds NEAR,
//!   and never while the relayer holds less than `MARVII_FUND_RESERVE_NEAR`
//!   (default 5).
//!
//! Counters live in memory; a restart resets them, which the on-chain checks
//! (balance, signature, nonce) don't depend on.

use std::{collections::HashMap, collections::HashSet, sync::Mutex, time::{SystemTime, UNIX_EPOCH}};

fn env_num(k: &str, d: u64) -> u64 {
    std::env::var(k).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(d)
}

pub struct Limits {
    pub wallet_day: u32,
    pub hour: u32,
    pub fund_day: u32,
    pub fund_reserve_near: u64,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    day: u64,
    per_wallet: HashMap<String, u32>,
    funds: u32,
    fund_ips: HashSet<String>,
    hour: u64,
    hour_count: u32,
}

fn now_s() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl State {
    fn roll(&mut self, t: u64) {
        if self.day != t / 86_400 {
            self.day = t / 86_400;
            self.per_wallet.clear();
            self.funds = 0;
            self.fund_ips.clear();
        }
        if self.hour != t / 3_600 {
            self.hour = t / 3_600;
            self.hour_count = 0;
        }
    }
}

impl Limits {
    pub fn from_env() -> Self {
        Self::new(
            env_num("MARVII_LIMIT_WALLET_DAY", 50) as u32,
            env_num("MARVII_LIMIT_HOUR", 300) as u32,
            env_num("MARVII_FUND_DAY", 20) as u32,
            env_num("MARVII_FUND_RESERVE_NEAR", 5),
        )
    }

    pub fn new(wallet_day: u32, hour: u32, fund_day: u32, fund_reserve_near: u64) -> Self {
        Self { wallet_day, hour, fund_day, fund_reserve_near, state: Mutex::new(State::default()) }
    }

    /// Counts one paid transaction for `wallet`, or refuses it.
    pub fn charge(&self, wallet: &str) -> Result<(), String> {
        self.charge_at(wallet, now_s())
    }

    fn charge_at(&self, wallet: &str, t: u64) -> Result<(), String> {
        let mut s = self.state.lock().unwrap();
        s.roll(t);
        if s.hour_count >= self.hour {
            return Err("the relayer is busy; try again within the hour".into());
        }
        let used = s.per_wallet.get(wallet).copied().unwrap_or(0);
        if used >= self.wallet_day {
            return Err(format!("this account has used its {} relayed transactions for today", self.wallet_day));
        }
        s.per_wallet.insert(wallet.to_string(), used + 1);
        s.hour_count += 1;
        Ok(())
    }

    /// Reserves one new-wallet gift for `ip`, or refuses. `undo_fund` gives
    /// it back if the transfer then fails.
    pub fn take_fund(&self, ip: &str) -> Result<(), String> {
        self.take_fund_at(ip, now_s())
    }

    fn take_fund_at(&self, ip: &str, t: u64) -> Result<(), String> {
        let mut s = self.state.lock().unwrap();
        s.roll(t);
        if s.funds >= self.fund_day {
            return Err("today's new-wallet NEAR is used up; try again tomorrow".into());
        }
        if !s.fund_ips.insert(ip.to_string()) {
            return Err("one new wallet per connection per day".into());
        }
        s.funds += 1;
        Ok(())
    }

    pub fn undo_fund(&self, ip: &str) {
        let mut s = self.state.lock().unwrap();
        s.fund_ips.remove(ip);
        s.funds = s.funds.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_wallet_and_hourly_caps() {
        let l = Limits::new(2, 3, 1, 5);
        let t = 1_800_000_000;
        assert!(l.charge_at("a", t).is_ok());
        assert!(l.charge_at("a", t).is_ok());
        assert!(l.charge_at("a", t).unwrap_err().contains("used its 2"));
        assert!(l.charge_at("b", t).is_ok());
        assert!(l.charge_at("c", t).unwrap_err().contains("busy"));
        // A new hour frees the global cap, not the wallet's day.
        assert!(l.charge_at("c", t + 3_600).is_ok());
        assert!(l.charge_at("a", t + 3_600).is_err());
        // A new day frees both.
        assert!(l.charge_at("a", t + 86_400).is_ok());
    }

    #[test]
    fn funding_caps() {
        let l = Limits::new(50, 300, 2, 5);
        let t = 1_800_000_000;
        assert!(l.take_fund_at("1.1.1.1", t).is_ok());
        assert!(l.take_fund_at("1.1.1.1", t).unwrap_err().contains("per connection"));
        l.undo_fund("1.1.1.1");
        assert!(l.take_fund_at("1.1.1.1", t).is_ok());
        assert!(l.take_fund_at("2.2.2.2", t).is_ok());
        assert!(l.take_fund_at("3.3.3.3", t).unwrap_err().contains("used up"));
        assert!(l.take_fund_at("3.3.3.3", t + 86_400).is_ok());
    }
}
