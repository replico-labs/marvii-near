//! The name registry, deployed at the parent account (`marvii.testnet`).
//!
//! A Marvii wallet calls `register("alice")` itself, so the call proves who
//! owns the name: only the wallet's passkey can make the wallet call. The
//! registry creates `alice.marvii.testnet` with the forwarder's shared
//! (global) code, no access keys, and the wallet as the forwarder's only
//! target. Wallet apps and exchanges accept an ordinary name like this as a
//! recipient; the `0s…` wallet address itself they don't yet recognise.
//!
//! One name per wallet, first come first served; a name can never be
//! reassigned. There are no admin methods.

use near_sdk::json_types::Base58CryptoHash;
use near_sdk::store::{LookupMap, Vector};
use near_sdk::{AccountId, CryptoHash, Gas, NearToken, PanicOnDefault, Promise, PromiseError, env, near, require};

/// What the registry gives each name's account for its storage. The
/// forwarder passes on anything above what it needs to the wallet.
pub const NAME_FUNDING: NearToken = NearToken::from_millinear(5);
const INIT_GAS: Gas = Gas::from_tgas(5);
const CALLBACK_GAS: Gas = Gas::from_tgas(5);

/// Names that would let someone pose as us.
const RESERVED: &[&str] = &["marvii", "admin", "support", "help", "official", "team", "wallet", "near", "security", "relayer"];

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Names {
    forwarder_code_hash: CryptoHash,
    /// wallet → name (the short label, e.g. "alice")
    by_wallet: LookupMap<AccountId, String>,
    /// name → wallet
    by_name: LookupMap<String, AccountId>,
    /// Every name created, in order, for the relayer's forwarding loop.
    list: Vector<String>,
}

#[near]
impl Names {
    #[init]
    #[private]
    pub fn new(forwarder_code_hash: Base58CryptoHash) -> Self {
        Self {
            forwarder_code_hash: forwarder_code_hash.into(),
            by_wallet: LookupMap::new(b"w"),
            by_name: LookupMap::new(b"n"),
            list: Vector::new(b"l"),
        }
    }

    /// Registers `name` for the calling wallet and creates its deposit
    /// address. Only `0s…` (NEP-616) accounts, i.e. contract wallets, may
    /// register, one name each.
    pub fn register(&mut self, name: String) -> Promise {
        let wallet = env::predecessor_account_id();
        require!(wallet.as_str().starts_with("0s"), "only Marvii wallets register names");
        require!(!self.by_wallet.contains_key(&wallet), "this wallet already has a name");
        check_name(&name);
        require!(!self.by_name.contains_key(&name), "that name is taken");
        let account: AccountId = format!("{name}.{}", env::current_account_id()).parse().unwrap_or_else(|_| env::panic_str("invalid name"));
        self.by_wallet.insert(wallet.clone(), name.clone());
        self.by_name.insert(name.clone(), wallet.clone());
        let init = near_sdk::serde_json::json!({ "target": wallet }).to_string().into_bytes();
        Promise::new(account)
            .create_account()
            .transfer(NAME_FUNDING)
            .use_global_contract(self.forwarder_code_hash)
            .function_call("new", init, NearToken::from_yoctonear(0), INIT_GAS)
            .then(Self::ext(env::current_account_id()).with_static_gas(CALLBACK_GAS).on_created(name, wallet))
    }

    /// Keeps the name if its account was created; otherwise frees it again.
    #[private]
    pub fn on_created(&mut self, name: String, wallet: AccountId, #[callback_result] created: Result<(), PromiseError>) -> bool {
        // #[private] already checks this; repeated so a test can reach it.
        require!(env::predecessor_account_id() == env::current_account_id(), "only the registry");
        if created.is_ok() {
            env::log_str(&format!("name {name} → {wallet}"));
            self.list.push(name);
            true
        } else {
            self.by_wallet.remove(&wallet);
            self.by_name.remove(&name);
            false
        }
    }

    /// The wallet's name as a full account, e.g. `alice.marvii.testnet`.
    pub fn name_of(&self, wallet: AccountId) -> Option<AccountId> {
        self.by_wallet.get(&wallet).map(|n| full(n))
    }

    /// The wallet a name forwards to.
    pub fn wallet_of(&self, name: String) -> Option<AccountId> {
        let label = name.strip_suffix(&format!(".{}", env::current_account_id())).unwrap_or(&name).to_string();
        self.by_name.get(&label).cloned()
    }

    /// Created names as full accounts, from `from_index`, at most `limit` (≤ 100).
    pub fn names(&self, from_index: u32, limit: u32) -> Vec<AccountId> {
        self.list.iter().skip(from_index as usize).take(limit.min(100) as usize).map(|n| full(n)).collect()
    }

    pub fn count(&self) -> u32 {
        self.list.len()
    }
}

fn full(label: &str) -> AccountId {
    format!("{label}.{}", env::current_account_id()).parse().unwrap()
}

/// 2–32 characters: lowercase letters and digits, with single `-` or `_`
/// between them.
fn check_name(name: &str) {
    let b = name.as_bytes();
    let ok = (2..=32).contains(&b.len())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-' || *c == b'_')
        && b[0].is_ascii_alphanumeric()
        && b[b.len() - 1].is_ascii_alphanumeric()
        && !b.windows(2).any(|w| !w[0].is_ascii_alphanumeric() && !w[1].is_ascii_alphanumeric());
    require!(ok, "a name is 2–32 lowercase letters and digits, with single - or _ between them");
    require!(!RESERVED.contains(&name), "that name is reserved");
    // Names that look like other account kinds (0x…, 0s…) are refused too.
    require!(!(name.starts_with("0x") || name.starts_with("0s")), "that name is reserved");
}

#[cfg(test)]
mod tests;
