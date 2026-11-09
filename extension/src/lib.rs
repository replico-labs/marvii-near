//! Marvii session extension for `defuse-wallet`.
//!
//! The wallet's owner (the passkey) installs this contract as an extension
//! and grants session keys to agents and apps. A session key can only make
//! the wallet act within its grant:
//!
//! - NEAR calls to listed venues (contract + method), NEP-141 transfers to
//!   listed recipients, NEP-141 `ft_transfer_call` only to listed venues
//! - EVM transactions through Chain Signatures: calls to listed venues
//!   (chain + contract + selector), ERC-20 `approve` only to a listed venue,
//!   ERC-20 `transfer` and native transfers only to listed recipients
//! - every amount counted against a daily cap per asset; an asset without a
//!   cap can't be moved at all
//! - an expiry and an inactivity expiry
//!
//! **An extension has the same power as the passkey over every wallet that
//! installs it** (docs/near.md). So this contract never forwards anything it
//! hasn't decoded and checked, never sends wallet ops (add/remove extension,
//! signature mode), and never lets a session call the wallet, this
//! extension, or the MPC signer directly. Testnet only: unaudited.

pub mod evm;
mod hl;
mod utxo;
pub mod rules;

use defuse_wallet::{
    NearPromise, Request,
    actions::FunctionCall,
};
use near_sdk::{
    AccountId, Gas, NearToken, PanicOnDefault, Promise, env, json_types::U128, log, near,
    require, store::LookupMap,
};

pub use rules::{Action, EvmTx, Grant, Mode, Session};

/// Signed session messages start with this, so a signature for anything
/// else can't be replayed here.
pub const DOMAIN: &str = "marvii-session-v1";
/// How far ahead a message's deadline may be.
pub const MAX_DEADLINE_MS: u64 = 10 * 60 * 1000;
/// A grant may last at most 90 days.
pub const MAX_GRANT_MS: u64 = 90 * 24 * 60 * 60 * 1000;
/// Deposit a wallet attaches when granting, towards storage.
pub const GRANT_DEPOSIT: NearToken = NearToken::from_millinear(10);
/// Most queued requests per wallet at once.
pub const MAX_QUEUED: usize = 16;
/// The shortest delay an extension may be deployed with.
pub const MIN_QUEUE_DELAY_MS: u64 = 60 * 1000;
/// Most session keys per wallet (revoked ones count; re-grant to reuse).
pub const MAX_SESSIONS: usize = 32;
/// Gas for the MPC signer's `sign` (it needs at least 15 TGas attached).
pub const SIGN_GAS: Gas = Gas::from_tgas(30);
/// Gas for a NEAR venue call.
pub const CALL_GAS: Gas = Gas::from_tgas(60);

/// The signed message an agent sends. The JSON text itself is what's
/// signed: `ed25519(sha256(msg))`.
#[near(serializers = [json])]
#[derive(Clone, Debug)]
pub struct SessionMessage {
    pub domain: String,
    pub extension: AccountId,
    pub wallet: AccountId,
    pub key_id: String,
    pub nonce: u64,
    pub deadline_ms: u64,
    pub action: Action,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Extension {
    /// May pause and unpause sessions. It can't grant anything.
    owner: AccountId,
    paused: bool,
    /// The Chain Signatures contract (v1.signer-prod.testnet on testnet).
    signer: AccountId,
    sessions: LookupMap<(AccountId, String), Session>,
    /// Each wallet's key IDs, so the dashboard can list them.
    keys: LookupMap<AccountId, Vec<String>>,
    /// How long a request to a new recipient or venue waits, during which
    /// the passkey can cancel it. Fixed at deployment.
    queue_delay_ms: u64,
    queue: LookupMap<u64, Queued>,
    queue_ids: LookupMap<AccountId, Vec<u64>>,
    next_queue_id: u64,
}

/// A request waiting out the delay.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug)]
pub struct Queued {
    pub wallet: AccountId,
    pub key_id: String,
    /// The action as the agent signed it (JSON), checked again on execution.
    pub action: String,
    pub created_ms: u64,
    pub ready_at_ms: u64,
}

#[near]
impl Extension {
    #[init]
    pub fn new(owner: AccountId, signer: AccountId, queue_delay_ms: u64) -> Self {
        require!(queue_delay_ms >= MIN_QUEUE_DELAY_MS, "queue delay must be at least a minute");
        Self {
            owner,
            paused: false,
            signer,
            sessions: LookupMap::new(b"s"),
            keys: LookupMap::new(b"k"),
            queue_delay_ms,
            queue: LookupMap::new(b"q"),
            queue_ids: LookupMap::new(b"w"),
            next_queue_id: 1,
        }
    }

    // ---- Called by the wallet itself (the passkey signed the request) ----

    /// Grants a session key. The caller is the wallet: only a request the
    /// passkey signed (or another enabled extension) can make it call this.
    #[payable]
    pub fn grant_session(&mut self, key_id: String, grant: Grant) {
        require!(env::attached_deposit() >= GRANT_DEPOSIT, "attach 0.01 NEAR for storage");
        let wallet = env::predecessor_account_id();
        let now = env::block_timestamp_ms();
        if let Err(e) = rules::validate_grant(&grant, &key_id, now, &wallet, &env::current_account_id(), &self.signer) {
            env::panic_str(e);
        }
        let key = (wallet.clone(), key_id.clone());
        // Keep the nonce across re-grants, so old messages never replay.
        let nonce = self.sessions.get(&key).map(|s| s.nonce).unwrap_or(0);
        let ks = self.keys.entry(wallet.clone()).or_default();
        if !ks.contains(&key_id) {
            require!(ks.len() < MAX_SESSIONS, "too many session keys; reuse a key ID");
            ks.push(key_id.clone());
        }
        self.sessions.insert(key, Session::new(grant, nonce, now));
        log!("session_granted wallet={} key_id={}", wallet, key_id);
    }

    /// Revokes a session key. Its nonce is kept.
    pub fn revoke_session(&mut self, key_id: String) {
        let wallet = env::predecessor_account_id();
        let key = (wallet.clone(), key_id.clone());
        if let Some(s) = self.sessions.get_mut(&key) {
            s.revoked = true;
            log!("session_revoked wallet={} key_id={}", wallet, key_id);
        }
    }

    /// Cancels a queued request. Only the wallet (the passkey) can.
    pub fn cancel_queued(&mut self, id: u64) {
        let wallet = env::predecessor_account_id();
        let q = self.queue.get(&id).unwrap_or_else(|| env::panic_str("no such queued request"));
        require!(q.wallet == wallet, "not your request");
        self.unqueue(id, &wallet);
        log!("queued_cancelled wallet={} id={}", wallet, id);
    }

    // ---- Called by anyone (a relayer) with an agent's signed message ----

    /// Checks an agent's signed message and, if every rule passes, has the
    /// wallet act. Attach at least 1 yoctoNEAR: the wallet requires a deposit
    /// on extension calls, and it funds the MPC signer's 1 yoctoNEAR.
    #[payable]
    pub fn execute(&mut self, msg: String, signature: String) -> Promise {
        let deposit = Self::require_deposit();
        let (m, now) = self.authenticate(&msg, &signature);
        let session = self.sessions.get_mut(&(m.wallet.clone(), m.key_id.clone())).unwrap();
        let plan = rules::check(session, &m.action, Mode::Direct, now, &m.wallet, &env::current_account_id(), &self.signer)
            .unwrap_or_else(|e| env::panic_str(e));
        session.record(&plan.spends, now);
        log!("session_used wallet={} key_id={} nonce={}", m.wallet, m.key_id, m.nonce);
        self.act(m.wallet, plan, deposit)
    }

    /// An agent proposes a payment to a new recipient or a call to a new
    /// contract. Every other rule is checked now and again on execution; it
    /// waits `queue_delay_ms`, and the passkey can cancel it until then.
    pub fn propose(&mut self, msg: String, signature: String) -> u64 {
        let (m, now) = self.authenticate(&msg, &signature);
        let session = self.sessions.get(&(m.wallet.clone(), m.key_id.clone())).unwrap();
        if let Err(e) = rules::check(session, &m.action, Mode::Queued, now, &m.wallet, &env::current_account_id(), &self.signer) {
            env::panic_str(e);
        }
        let ids = self.queue_ids.entry(m.wallet.clone()).or_default();
        require!(ids.len() < MAX_QUEUED, "too many queued requests for this wallet");
        let id = self.next_queue_id;
        self.next_queue_id += 1;
        ids.push(id);
        let ready_at_ms = now + self.queue_delay_ms;
        let action = serde_json::to_string(&m.action).unwrap();
        self.queue.insert(id, Queued { wallet: m.wallet.clone(), key_id: m.key_id.clone(), action, created_ms: now, ready_at_ms });
        log!("queued wallet={} key_id={} id={} ready_at_ms={}", m.wallet, m.key_id, id, ready_at_ms);
        id
    }

    /// Runs a queued request once its delay has passed. Anyone may call it;
    /// the session's rules are checked again now, so a revoked or expired
    /// session, a pause, or a used-up cap still stops it.
    #[payable]
    pub fn execute_queued(&mut self, id: u64) -> Promise {
        let deposit = Self::require_deposit();
        require!(!self.paused, "sessions are paused");
        let q = self.queue.get(&id).cloned().unwrap_or_else(|| env::panic_str("no such queued request"));
        let now = env::block_timestamp_ms();
        require!(now >= q.ready_at_ms, "not ready yet");
        let action: Action = serde_json::from_str(&q.action).unwrap();
        let session = self.sessions.get_mut(&(q.wallet.clone(), q.key_id.clone())).unwrap_or_else(|| env::panic_str("unknown session"));
        let plan = rules::check(session, &action, Mode::Queued, now, &q.wallet, &env::current_account_id(), &self.signer)
            .unwrap_or_else(|e| env::panic_str(e));
        session.record(&plan.spends, now);
        session.last_used_ms = now;
        self.unqueue(id, &q.wallet);
        log!("queued_executed wallet={} key_id={} id={}", q.wallet, q.key_id, id);
        self.act(q.wallet, plan, deposit)
    }

    // ---- Owner: pause only restricts ----

    pub fn pause(&mut self) {
        require!(env::predecessor_account_id() == self.owner, "owner only");
        self.paused = true;
    }

    pub fn unpause(&mut self) {
        require!(env::predecessor_account_id() == self.owner, "owner only");
        self.paused = false;
    }

    // ---- Views ----

    pub fn list_queued(&self, wallet: AccountId) -> Vec<(u64, Queued)> {
        self.queue_ids
            .get(&wallet)
            .map(|ids| ids.iter().filter_map(|id| self.queue.get(id).map(|q| (*id, q.clone()))).collect())
            .unwrap_or_default()
    }

    pub fn queue_delay_ms(&self) -> u64 {
        self.queue_delay_ms
    }

    pub fn get_session(&self, wallet: AccountId, key_id: String) -> Option<Session> {
        self.sessions.get(&(wallet, key_id)).cloned()
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// The derivation path used for every EVM chain (one address on all).
    pub fn evm_path(&self) -> String {
        rules::EVM_PATH.to_string()
    }

    /// A wallet's sessions, newest grant last.
    pub fn list_sessions(&self, wallet: AccountId) -> Vec<(String, Session)> {
        self.keys
            .get(&wallet)
            .map(|ks| ks.iter().filter_map(|k| self.sessions.get(&(wallet.clone(), k.clone())).map(|s| (k.clone(), s.clone()))).collect())
            .unwrap_or_default()
    }

    /// The hash the wallet would ask the MPC signer to sign for this
    /// transaction, so the relayer can attach the returned signature.
    pub fn evm_signing_hash(&self, tx: EvmTx) -> String {
        let t = tx.parse().unwrap_or_else(|e| env::panic_str(e));
        hex::encode(t.signing_hash())
    }

    /// The EIP-712 digest a Hyperliquid action is signed over (for the
    /// agent, which checks the signature recovers to the account's address).
    pub fn hl_signing_hash(&self, action: Action) -> String {
        let nonce = match &action {
            Action::HlOrder { nonce, .. } | Action::HlCancel { nonce, .. } | Action::HlLeverage { nonce, .. } => *nonce,
            _ => env::panic_str("not a Hyperliquid action"),
        };
        let bytes = match &action {
            Action::HlOrder { asset, is_buy, price, size, reduce_only, tif, .. } => {
                hl::parse_decimal(price).and_then(|_| hl::parse_decimal(size)).unwrap_or_else(|e| env::panic_str(e));
                hl::order_action(*asset, *is_buy, price, size, *reduce_only, tif)
            }
            Action::HlCancel { asset, oid, .. } => hl::cancel_action(*asset, *oid),
            Action::HlLeverage { asset, is_cross, leverage, .. } => hl::leverage_action(*asset, *is_cross, *leverage),
            _ => unreachable!(),
        };
        hex::encode(hl::signing_hash(&bytes, nonce))
    }

    pub fn spent_today(&self, wallet: AccountId, key_id: String, asset: String) -> U128 {
        let now = env::block_timestamp_ms();
        U128(self.sessions.get(&(wallet, key_id)).map(|s| s.spent_on(&asset, now)).unwrap_or(0))
    }
}

impl Extension {
    fn require_deposit() -> NearToken {
        let deposit = env::attached_deposit();
        require!(!deposit.is_zero(), "attach at least 1 yoctoNEAR");
        deposit
    }

    /// Checks the message, its signature, nonce and deadline, and advances
    /// the session's nonce. Every state change reverts if the call panics.
    fn authenticate(&mut self, msg: &str, signature: &str) -> (SessionMessage, u64) {
        require!(!self.paused, "sessions are paused");
        let m: SessionMessage = serde_json::from_str(msg).unwrap_or_else(|_| env::panic_str("bad message"));
        require!(m.domain == DOMAIN, "wrong domain");
        require!(m.extension == env::current_account_id(), "message is for another extension");
        let now = env::block_timestamp_ms();
        let session = self.sessions.get_mut(&(m.wallet.clone(), m.key_id.clone())).unwrap_or_else(|| env::panic_str("unknown session"));
        // The signature covers the exact message text.
        let sig: [u8; 64] = hex::decode(signature).ok().and_then(|v| v.try_into().ok())
            .unwrap_or_else(|| env::panic_str("signature must be 64 bytes hex"));
        let digest = env::sha256_array(msg.as_bytes());
        require!(env::ed25519_verify(&sig, &digest, &session.public_key()), "bad signature");
        require!(m.nonce == session.nonce + 1, "wrong nonce");
        require!(m.deadline_ms >= now && m.deadline_ms <= now + MAX_DEADLINE_MS, "deadline passed or too far ahead");
        session.nonce = m.nonce;
        session.last_used_ms = now;
        (m, now)
    }

    fn unqueue(&mut self, id: u64, wallet: &AccountId) {
        self.queue.remove(&id);
        if let Some(ids) = self.queue_ids.get_mut(wallet) {
            ids.retain(|i| *i != id);
        }
    }

    /// Has the wallet carry out a checked plan. Never any wallet ops: a
    /// session can't add or remove extensions or change the signature mode.
    fn act(&self, wallet: AccountId, plan: rules::Plan, deposit: NearToken) -> Promise {
        let sign = |path: &str, hash: [u8; 32]| {
            NearPromise::new(self.signer.clone()).function_call(
                FunctionCall::name("sign")
                    .args_json(serde_json::json!({
                        "request": { "path": path, "payload_v2": { "Ecdsa": hex::encode(hash) }, "domain_id": 0 }
                    }))
                    .attach_deposit(NearToken::from_yoctonear(1))
                    .gas(SIGN_GAS),
            )
        };
        if let rules::Kind::Utxo { path, hashes } = &plan.kind {
            // One signature per input.
            let request = Request { internal: vec![], external: hashes.iter().map(|h| sign(path, *h)).collect() };
            return Promise::new(wallet).function_call(
                "w_execute_extension".to_string(),
                serde_json::to_vec(&serde_json::json!({ "request": request })).unwrap(),
                deposit,
                Gas::from_tgas(100),
            );
        }
        let promise = match plan.kind {
            rules::Kind::Near { receiver_id, method, args, deposit } => NearPromise::new(receiver_id)
                .function_call(FunctionCall::name(method).args(args).attach_deposit(deposit).gas(CALL_GAS)),
            rules::Kind::Evm { path, hash } => sign(&path, hash),
            rules::Kind::Utxo { .. } => unreachable!(),
        };
        let request = Request { internal: vec![], external: vec![promise] };
        Promise::new(wallet).function_call(
            "w_execute_extension".to_string(),
            serde_json::to_vec(&serde_json::json!({ "request": request })).unwrap(),
            deposit,
            Gas::from_tgas(100),
        )
    }
}

#[cfg(test)]
mod tests;
