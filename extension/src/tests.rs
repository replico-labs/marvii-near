//! Each rule gets a test that attempts the attack with a session key and
//! expects it to fail.

use super::*;
use crate::rules::{Cap, EvmVenue, NearVenue};
use ed25519_dalek::{Signer, SigningKey};
use near_sdk::{test_utils::VMContextBuilder, testing_env};
use sha2::{Digest, Sha256};

const NOW: u64 = 1_800_000_000_000;
const DAY: u64 = 24 * 60 * 60 * 1000;
const BASE_SEPOLIA: u64 = 84532;
const POOL: &str = "0x2222222222222222222222222222222222222222";
const USDC: &str = "0x4444444444444444444444444444444444444444";
const FRIEND: &str = "0x5555555555555555555555555555555555555555";
const STRANGER: &str = "0x6666666666666666666666666666666666666666";
/// Aave `supply(address,uint256,address,uint16)`, computed with `cast sig`.
const DELAY: u64 = 60 * 60 * 1000;
const SUPPLY: &str = "0x617ba037";

fn ext() -> AccountId { "ext.testnet".parse().unwrap() }
fn wallet() -> AccountId { "wallet.testnet".parse().unwrap() }
fn signer() -> AccountId { "v1.signer-prod.testnet".parse().unwrap() }
fn owner() -> AccountId { "owner.testnet".parse().unwrap() }
fn dex() -> AccountId { "dex.testnet".parse().unwrap() }
fn token() -> AccountId { "usdc.testnet".parse().unwrap() }
fn agent() -> SigningKey { SigningKey::from_bytes(&[7u8; 32]) }

fn ctx(predecessor: AccountId, deposit: NearToken, now: u64) {
    testing_env!(VMContextBuilder::new()
        .current_account_id(ext())
        .predecessor_account_id(predecessor)
        .attached_deposit(deposit)
        .block_timestamp(now * 1_000_000)
        .build());
}

fn grant() -> Grant {
    Grant {
        public_key: hex::encode(agent().verifying_key().to_bytes()),
        label: "Trading agent".into(),
        expires_at_ms: NOW + 7 * DAY,
        inactivity_ms: 3 * DAY,
        near_venues: vec![NearVenue { receiver_id: dex(), method: "swap".into() }],
        evm_venues: vec![EvmVenue { chain_id: BASE_SEPOLIA, to: POOL.into(), selector: SUPPLY.into() }],
        near_recipients: vec![],
        evm_recipients: vec![format!("{BASE_SEPOLIA}:{FRIEND}")],
        caps: vec![
            Cap { asset: format!("evm:{BASE_SEPOLIA}:{USDC}"), daily: U128(50_000_000) },
            Cap { asset: "near".into(), daily: U128(NearToken::from_near(1).as_yoctonear()) },
            Cap { asset: format!("nep141:{}", token()), daily: U128(100) },
        ],
        queue_new_targets: false,
    }
}

fn setup() -> Extension {
    ctx(owner(), NearToken::ZERO, NOW);
    let mut c = Extension::new(owner(), signer(), DELAY);
    ctx(wallet(), GRANT_DEPOSIT, NOW);
    c.grant_session("agent-1".into(), grant());
    c
}

fn sign_msg(nonce: u64, action: serde_json::Value) -> (String, String) {
    let msg = serde_json::json!({
        "domain": DOMAIN, "extension": ext(), "wallet": wallet(), "key_id": "agent-1",
        "nonce": nonce, "deadline_ms": NOW + 60_000, "action": action
    })
    .to_string();
    let sig = agent().sign(&Sha256::digest(msg.as_bytes()));
    (msg, hex::encode(sig.to_bytes()))
}

fn word_addr(a: &str) -> String { format!("{:0>64}", a.trim_start_matches("0x")) }
fn word_u(v: u128) -> String { format!("{v:064x}") }

fn evm(to: &str, value: u128, data: String) -> serde_json::Value {
    serde_json::json!({ "type": "evm_tx", "tx": {
        "chain_id": BASE_SEPOLIA, "nonce": 0, "max_priority_fee_per_gas": "1000000",
        "max_fee_per_gas": "2000000000", "gas_limit": 300000, "to": to,
        "value": value.to_string(), "data": data
    }})
}
fn approve(spender: &str, amount: u128) -> serde_json::Value {
    evm(USDC, 0, format!("0x095ea7b3{}{}", word_addr(spender), word_u(amount)))
}
fn supply(amount: u128) -> serde_json::Value {
    evm(POOL, 0, format!("{SUPPLY}{}{}{}{}", word_addr(USDC), word_u(amount), word_addr("0x00"), word_u(0)))
}

fn exec(c: &mut Extension, nonce: u64, action: serde_json::Value) {
    let (m, s) = sign_msg(nonce, action);
    ctx("relayer.testnet".parse().unwrap(), NearToken::from_yoctonear(1), NOW + 1000);
    let _ = c.execute(m, s);
}

/// Runs `check` directly, for the rule matrix.
fn try_action(c: &Extension, action: serde_json::Value, now: u64) -> Result<(), &'static str> {
    let s = c.get_session(wallet(), "agent-1".into()).unwrap();
    let a: Action = serde_json::from_value(action).unwrap();
    rules::check(&s, &a, Mode::Direct, now, &wallet(), &ext(), &signer()).map(|_| ())
}

// ---- Encoding ----

#[test]
fn evm_signing_hash_matches_cast() {
    // Reference: `cast mktx --raw-unsigned --chain 84532 --nonce 7 --gas-limit 210000
    // --priority-gas-price 1000000 --gas-price 2000000000 --value 5 0x2222..22
    // "approve(address,uint256)" 0x3333..33 1000000`, then `cast keccak`.
    let tx = evm::Eip1559 {
        chain_id: 84532, nonce: 7, max_priority_fee_per_gas: 1_000_000, max_fee_per_gas: 2_000_000_000,
        gas_limit: 210_000, to: [0x22; 20], value: 5,
        data: hex::decode(format!("095ea7b3{}{}", word_addr("0x3333333333333333333333333333333333333333"), word_u(1_000_000))).unwrap(),
    };
    ctx(owner(), NearToken::ZERO, NOW);
    assert_eq!(
        format!("0x02{}", hex::encode(tx.rlp_payload())),
        "0x02f86f83014a3407830f424084773594008303345094222222222222222222222222222222222222222205b844095ea7b3000000000000000000000000333333333333333333333333333333333333333300000000000000000000000000000000000000000000000000000000000f4240c0"
    );
    assert_eq!(hex::encode(tx.signing_hash()), "743d877c584f6b318e1199aa38eecfdb73d463badffe53ec78c0a67ff0435ccd");
}

// ---- What a session may do ----

#[test]
fn allowed_actions_pass() {
    let c = setup();
    assert_eq!(try_action(&c, approve(POOL, 20_000_000), NOW), Ok(()));
    assert_eq!(try_action(&c, supply(20_000_000), NOW), Ok(()));
    assert_eq!(try_action(&c, evm(USDC, 0, format!("0xa9059cbb{}{}", word_addr(FRIEND), word_u(5))), NOW), Ok(()));
    let swap = serde_json::json!({ "type": "near_call", "receiver_id": dex(), "method": "swap", "args": "{}", "deposit": "0" });
    assert_eq!(try_action(&c, swap, NOW), Ok(()));
}

#[test]
fn execute_advances_nonce_and_counts_spend() {
    let mut c = setup();
    exec(&mut c, 1, approve(POOL, 20_000_000));
    let s = c.get_session(wallet(), "agent-1".into()).unwrap();
    assert_eq!(s.nonce, 1);
    assert_eq!(c.spent_today(wallet(), "agent-1".into(), format!("evm:{BASE_SEPOLIA}:{USDC}")).0, 20_000_000);
    exec(&mut c, 2, approve(POOL, 30_000_000)); // exactly the 50 USDC cap
}

// ---- Attacks: each must fail ----

#[test]
fn over_the_daily_cap() {
    let mut c = setup();
    exec(&mut c, 1, approve(POOL, 40_000_000));
    assert_eq!(try_action(&c, approve(POOL, 10_000_001), NOW + 1000), Err("over the daily cap"));
    // The next day the cap is fresh.
    assert_eq!(try_action(&c, approve(POOL, 50_000_000), NOW + DAY), Ok(()));
}

#[test]
fn asset_without_a_cap_cant_move() {
    let c = setup();
    let other = "0x7777777777777777777777777777777777777777";
    assert_eq!(try_action(&c, evm(other, 0, format!("0x095ea7b3{}{}", word_addr(POOL), word_u(1))), NOW), Err("no cap for this asset"));
    assert_eq!(try_action(&c, evm(FRIEND, 1, "0x".into()), NOW), Err("no cap for this asset"));
}

#[test]
fn not_a_venue() {
    let c = setup();
    assert_eq!(try_action(&c, evm(STRANGER, 0, format!("{SUPPLY}{}", word_u(1))), NOW), Err("not an allowed venue"));
    // The right contract with another function.
    assert_eq!(try_action(&c, evm(POOL, 0, "0x69328dec".into()), NOW), Err("not an allowed venue"));
    // Same contract on another chain.
    let mut a = supply(1);
    a["tx"]["chain_id"] = 1.into();
    assert_eq!(try_action(&c, a, NOW), Err("not an allowed venue"));
    let call = serde_json::json!({ "type": "near_call", "receiver_id": dex(), "method": "withdraw_all", "args": "{}", "deposit": "0" });
    assert_eq!(try_action(&c, call, NOW), Err("not an allowed venue"));
}

#[test]
fn approve_to_a_non_venue() {
    let c = setup();
    assert_eq!(try_action(&c, approve(STRANGER, 1), NOW), Err("spender is not a venue"));
}

#[test]
fn transfer_to_a_stranger() {
    let c = setup();
    let t = evm(USDC, 0, format!("0xa9059cbb{}{}", word_addr(STRANGER), word_u(1)));
    assert_eq!(try_action(&c, t, NOW), Err("recipient not allowed"));
    assert_eq!(try_action(&c, evm(STRANGER, 1, "0x".into()), NOW), Err("recipient not allowed"));
}

#[test]
fn session_cant_call_the_wallet_the_extension_or_the_signer() {
    let c = setup();
    for r in [wallet(), ext(), signer()] {
        let a = serde_json::json!({ "type": "near_call", "receiver_id": r, "method": "grant_session", "args": "{}", "deposit": "0" });
        assert_eq!(try_action(&c, a, NOW), Err("venue can't be the wallet, the extension or the signer"));
    }
}

#[test]
fn grant_cant_list_the_wallet_extension_or_signer_or_token_moves() {
    ctx(owner(), NearToken::ZERO, NOW);
    let mut g = grant();
    g.near_venues.push(NearVenue { receiver_id: ext(), method: "grant_session".into() });
    assert!(rules::validate_grant(&g, "k", NOW, &wallet(), &ext(), &signer()).is_err());
    let mut g = grant();
    g.evm_venues.push(EvmVenue { chain_id: BASE_SEPOLIA, to: USDC.into(), selector: "0x095ea7b3".into() });
    assert!(rules::validate_grant(&g, "k", NOW, &wallet(), &ext(), &signer()).is_err());
    let mut g = grant();
    g.expires_at_ms = NOW + 91 * DAY;
    assert!(rules::validate_grant(&g, "k", NOW, &wallet(), &ext(), &signer()).is_err());
}

#[test]
fn malformed_token_call_data() {
    let c = setup();
    // An approve with extra bytes after the arguments.
    let a = evm(USDC, 0, format!("0x095ea7b3{}{}00", word_addr(POOL), word_u(1)));
    assert!(try_action(&c, a, NOW).is_err());
    // A value sent along with an approve.
    let a = evm(USDC, 1, format!("0x095ea7b3{}{}", word_addr(POOL), word_u(1)));
    assert_eq!(try_action(&c, a, NOW), Err("approve with value"));
}

#[test]
fn expired_and_inactive_sessions() {
    let c = setup();
    assert_eq!(try_action(&c, supply(1), NOW + 7 * DAY), Err("session expired"));
    assert_eq!(try_action(&c, supply(1), NOW + 3 * DAY), Err("session expired from inactivity"));
}

#[test]
fn revoked_session() {
    let mut c = setup();
    ctx(wallet(), NearToken::ZERO, NOW);
    c.revoke_session("agent-1".into());
    assert_eq!(try_action(&c, supply(1), NOW), Err("session revoked"));
}

#[test]
#[should_panic(expected = "wrong nonce")]
fn replayed_nonce() {
    let mut c = setup();
    exec(&mut c, 1, supply(1));
    exec(&mut c, 1, supply(1));
}

#[test]
#[should_panic(expected = "wrong nonce")]
fn regrant_keeps_the_nonce() {
    let mut c = setup();
    exec(&mut c, 1, supply(1));
    ctx(wallet(), GRANT_DEPOSIT, NOW);
    c.grant_session("agent-1".into(), grant());
    exec(&mut c, 1, supply(1)); // the old message must not work again
}

#[test]
#[should_panic(expected = "bad signature")]
fn signature_from_another_key() {
    let mut c = setup();
    let (m, _) = sign_msg(1, supply(1));
    let other = SigningKey::from_bytes(&[9u8; 32]).sign(&Sha256::digest(m.as_bytes()));
    ctx("relayer.testnet".parse().unwrap(), NearToken::from_yoctonear(1), NOW + 1000);
    let _ = c.execute(m, hex::encode(other.to_bytes()));
}

#[test]
#[should_panic(expected = "bad signature")]
fn message_changed_after_signing() {
    let mut c = setup();
    let (m, s) = sign_msg(1, supply(1));
    let changed = m.replace("\"nonce\":1", "\"nonce\":1 ").replace(&word_u(1), &word_u(2));
    ctx("relayer.testnet".parse().unwrap(), NearToken::from_yoctonear(1), NOW + 1000);
    let _ = c.execute(changed, s);
}

#[test]
#[should_panic(expected = "message is for another extension")]
fn message_for_another_extension() {
    let mut c = setup();
    let msg = serde_json::json!({
        "domain": DOMAIN, "extension": "other.testnet", "wallet": wallet(), "key_id": "agent-1",
        "nonce": 1, "deadline_ms": NOW + 60_000, "action": supply(1)
    }).to_string();
    let sig = agent().sign(&Sha256::digest(msg.as_bytes()));
    ctx("relayer.testnet".parse().unwrap(), NearToken::from_yoctonear(1), NOW + 1000);
    let _ = c.execute(msg, hex::encode(sig.to_bytes()));
}

#[test]
#[should_panic(expected = "deadline passed or too far ahead")]
fn stale_message() {
    let mut c = setup();
    let (m, s) = sign_msg(1, supply(1));
    ctx("relayer.testnet".parse().unwrap(), NearToken::from_yoctonear(1), NOW + 120_000);
    let _ = c.execute(m, s);
}

#[test]
#[should_panic(expected = "sessions are paused")]
fn paused() {
    let mut c = setup();
    ctx(owner(), NearToken::ZERO, NOW);
    c.pause();
    exec(&mut c, 1, supply(1));
}

#[test]
#[should_panic(expected = "owner only")]
fn only_owner_pauses() {
    let mut c = setup();
    ctx(wallet(), NearToken::ZERO, NOW);
    c.unpause();
}

// ---- One EVM address, and the session index ----

#[test]
fn same_evm_path_on_every_chain() {
    let c = setup();
    let s = c.get_session(wallet(), "agent-1".into()).unwrap();
    for chain in [BASE_SEPOLIA, 11155111u64, 1] {
        let mut a = supply(1);
        a["tx"]["chain_id"] = chain.into();
        let mut g = s.clone();
        g.grant.evm_venues.push(EvmVenue { chain_id: chain, to: POOL.into(), selector: SUPPLY.into() });
        let a: Action = serde_json::from_value(a).unwrap();
        match rules::check(&g, &a, Mode::Direct, NOW, &wallet(), &ext(), &signer()).unwrap().kind {
            rules::Kind::Evm { path, .. } => assert_eq!(path, "evm"),
            _ => panic!("expected an EVM plan"),
        }
    }
    assert_eq!(c.evm_path(), "evm");
}

#[test]
fn list_sessions_by_wallet() {
    let mut c = setup();
    ctx(wallet(), GRANT_DEPOSIT, NOW);
    c.grant_session("agent-2".into(), grant());
    // Re-granting an existing key doesn't add a second entry.
    c.grant_session("agent-1".into(), grant());
    let l = c.list_sessions(wallet());
    assert_eq!(l.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["agent-1", "agent-2"]);
    assert!(c.list_sessions(owner()).is_empty());
    // Revoked sessions stay listed, marked revoked.
    c.revoke_session("agent-2".into());
    assert!(c.list_sessions(wallet())[1].1.revoked);
}

#[test]
#[should_panic(expected = "too many session keys")]
fn session_keys_per_wallet_are_capped() {
    let mut c = setup();
    ctx(wallet(), GRANT_DEPOSIT, NOW);
    for i in 2..=(MAX_SESSIONS + 1) {
        c.grant_session(format!("agent-{i}"), grant());
    }
}

// ---- The delay queue for new recipients and venues ----

fn queue_setup() -> Extension {
    let mut c = setup();
    let mut g = grant();
    g.queue_new_targets = true;
    ctx(wallet(), GRANT_DEPOSIT, NOW);
    c.grant_session("agent-1".into(), g);
    c
}

fn propose(c: &mut Extension, nonce: u64, action: serde_json::Value) -> u64 {
    let (m, s) = sign_msg(nonce, action);
    ctx("relayer.testnet".parse().unwrap(), NearToken::ZERO, NOW + 1000);
    c.propose(m, s)
}

fn run_queued(c: &mut Extension, id: u64, at: u64) {
    ctx("anyone.testnet".parse().unwrap(), NearToken::from_yoctonear(1), at);
    let _ = c.execute_queued(id);
}

fn stranger_transfer(amount: u128) -> serde_json::Value {
    evm(USDC, 0, format!("0xa9059cbb{}{}", word_addr(STRANGER), word_u(amount)))
}

#[test]
fn new_recipient_waits_then_runs() {
    let mut c = queue_setup();
    let id = propose(&mut c, 1, stranger_transfer(10_000_000));
    let q = c.list_queued(wallet());
    assert_eq!(q.len(), 1);
    assert_eq!(q[0].1.ready_at_ms, NOW + 1000 + DELAY);
    // Nothing spent yet.
    ctx(owner(), NearToken::ZERO, NOW + 2000);
    assert_eq!(c.spent_today(wallet(), "agent-1".into(), format!("evm:{BASE_SEPOLIA}:{USDC}")).0, 0);
    run_queued(&mut c, id, NOW + 1000 + DELAY);
    assert!(c.list_queued(wallet()).is_empty());
    ctx(owner(), NearToken::ZERO, NOW + 1000 + DELAY);
    assert_eq!(c.spent_today(wallet(), "agent-1".into(), format!("evm:{BASE_SEPOLIA}:{USDC}")).0, 10_000_000);
}

#[test]
#[should_panic(expected = "not ready yet")]
fn queued_cant_run_early() {
    let mut c = queue_setup();
    let id = propose(&mut c, 1, stranger_transfer(1));
    run_queued(&mut c, id, NOW + DELAY);
}

#[test]
#[should_panic(expected = "no such queued request")]
fn queued_runs_once() {
    let mut c = queue_setup();
    let id = propose(&mut c, 1, stranger_transfer(1));
    run_queued(&mut c, id, NOW + 1000 + DELAY);
    run_queued(&mut c, id, NOW + 2000 + DELAY);
}

#[test]
fn passkey_cancels() {
    let mut c = queue_setup();
    let id = propose(&mut c, 1, stranger_transfer(1));
    ctx(wallet(), NearToken::ZERO, NOW + 5000);
    c.cancel_queued(id);
    assert!(c.list_queued(wallet()).is_empty());
}

#[test]
#[should_panic(expected = "not your request")]
fn only_the_wallet_cancels() {
    let mut c = queue_setup();
    let id = propose(&mut c, 1, stranger_transfer(1));
    ctx("attacker.testnet".parse().unwrap(), NearToken::ZERO, NOW + 5000);
    c.cancel_queued(id);
}

#[test]
#[should_panic(expected = "can't queue new recipients")]
fn queue_needs_the_grant_to_allow_it() {
    let mut c = setup();
    propose(&mut c, 1, stranger_transfer(1));
}

#[test]
#[should_panic(expected = "session revoked")]
fn revoke_stops_a_queued_request() {
    let mut c = queue_setup();
    let id = propose(&mut c, 1, stranger_transfer(1));
    ctx(wallet(), NearToken::ZERO, NOW + 5000);
    c.revoke_session("agent-1".into());
    run_queued(&mut c, id, NOW + 1000 + DELAY);
}

#[test]
#[should_panic(expected = "over the daily cap")]
fn caps_still_apply_to_queued() {
    let mut c = queue_setup();
    propose(&mut c, 1, stranger_transfer(50_000_001));
}

#[test]
#[should_panic(expected = "over the daily cap")]
fn cap_is_checked_again_at_execution() {
    let mut c = queue_setup();
    let id = propose(&mut c, 1, stranger_transfer(40_000_000));
    // Meanwhile the agent uses the cap directly.
    exec(&mut c, 2, approve(POOL, 20_000_000));
    run_queued(&mut c, id, NOW + 1000 + DELAY);
}

#[test]
#[should_panic(expected = "venue can't be the wallet")]
fn queued_still_cant_call_the_wallet() {
    let mut c = queue_setup();
    propose(&mut c, 1, serde_json::json!({ "type": "near_call", "receiver_id": wallet(), "method": "w_execute_extension", "args": "{}", "deposit": "0" }));
}

#[test]
#[should_panic(expected = "sessions are paused")]
fn pause_stops_queued() {
    let mut c = queue_setup();
    let id = propose(&mut c, 1, stranger_transfer(1));
    ctx(owner(), NearToken::ZERO, NOW + 5000);
    c.pause();
    run_queued(&mut c, id, NOW + 1000 + DELAY);
}

#[test]
#[should_panic(expected = "queue delay must be at least a minute")]
fn delay_has_a_floor() {
    ctx(owner(), NearToken::ZERO, NOW);
    Extension::new(owner(), signer(), 59_999);
}
