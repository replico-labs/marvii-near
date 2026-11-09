//! Each rule gets a test that attempts the attack with a session key and
//! expects it to fail.

use super::*;
use crate::rules::{Cap, EvmVenue, NearVenue, UtxoGrant};
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
/// The account's own EVM address, as the grant names it.
const SELF: &str = "0x7777777777777777777777777777777777777777";
const ROUTER: &str = "0x8888888888888888888888888888888888888888";
const WETH: &str = "0x4200000000000000000000000000000000000006";
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
            // 100 USD of Hyperliquid orders a day.
            Cap { asset: rules::HL_CAP_ASSET.into(), daily: U128(100_000_000) },
        ],
        queue_new_targets: false,
        hl_markets: vec![rules::HlMarket { asset: 4, max_leverage: 5 }],
        evm_self: Some(SELF.into()),
        utxo: vec![],
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
    evm(POOL, 0, format!("{SUPPLY}{}{}{}{}", word_addr(USDC), word_u(amount), word_addr(SELF), word_u(0)))
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

// ---- Hyperliquid ----

/// Reference: hyperliquid-python-sdk 0.x (`action_hash`, `l1_payload`,
/// eth_account's EIP-712 hashing), testnet, nonce 1794197767416: an ETH
/// (asset 4) Gtc buy of 0.01 at 2000.5, a cancel of order 123456789, and
/// leverage 5x cross. MessagePack bytes and digests match.
#[test]
fn hl_digests_match_the_python_sdk() {
    ctx(owner(), NearToken::ZERO, NOW);
    let n = 1_794_197_767_416;
    let cases = [
        (hl::order_action(4, true, "2000.5", "0.01", false, "Gtc"), "83a474797065a56f72646572a66f72646572739186a16104a162c3a170a6323030302e35a173a4302e3031a172c2a17481a56c696d697481a3746966a3477463a867726f7570696e67a26e61", "49803b85581337b975464264e545835445dba9853026ec8394ef8e923eeac65e"),
        (hl::cancel_action(4, 123_456_789), "82a474797065a663616e63656ca763616e63656c739182a16104a16fce075bcd15", "0a19e326ac3e848246db125c624206e3645014e4e351f8f6baa0425da9887587"),
        (hl::leverage_action(4, true, 5), "84a474797065ae7570646174654c65766572616765a5617373657404a7697343726f7373c3a86c6576657261676505", "8ab50ff619c3e10354b3dd5470febc188aabdeb924962ea1980121ed106acb34"),
    ];
    for (bytes, want_mp, want_digest) in cases {
        assert_eq!(hex::encode(&bytes), want_mp);
        assert_eq!(hex::encode(hl::signing_hash(&bytes, n)), want_digest);
    }
}

fn hl_order(price: &str, size: &str, nonce: u64) -> serde_json::Value {
    serde_json::json!({ "type": "hl_order", "asset": 4, "is_buy": true, "price": price, "size": size, "reduce_only": false, "tif": "Gtc", "nonce": nonce })
}

#[test]
fn hl_order_within_the_grant_signs_its_digest() {
    let c = setup();
    let s = c.get_session(wallet(), "agent-1".into()).unwrap();
    let a: Action = serde_json::from_value(hl_order("2000.5", "0.01", NOW)).unwrap();
    let plan = rules::check(&s, &a, Mode::Direct, NOW, &wallet(), &ext(), &signer()).unwrap();
    let rules::Kind::Evm { path, hash } = plan.kind else { panic!("not a signature") };
    assert_eq!(path, "evm");
    assert_eq!(hash, hl::signing_hash(&hl::order_action(4, true, "2000.5", "0.01", false, "Gtc"), NOW));
    // 2000.5 × 0.01 = 20.005 USD.
    assert_eq!(plan.spends, vec![(rules::HL_CAP_ASSET.to_string(), 20_005_000)]);
}

#[test]
fn hl_attacks_are_refused() {
    let c = setup();
    // Another market.
    let mut other = hl_order("2000", "0.01", NOW);
    other["asset"] = 3.into();
    assert_eq!(try_action(&c, other, NOW), Err("Hyperliquid market not allowed"));
    // Over the daily cap: 2000 × 0.06 = 120 USD > 100.
    assert_eq!(try_action(&c, hl_order("2000", "0.06", NOW), NOW), Err("over the daily cap"));
    // A stale or far-future nonce: a signature for later use.
    assert!(try_action(&c, hl_order("2000", "0.01", NOW - 6 * 60 * 1000), NOW).is_err());
    assert!(try_action(&c, hl_order("2000", "0.01", NOW + 6 * 60 * 1000), NOW).is_err());
    // Non-canonical numbers (would hash differently from what Hyperliquid sees).
    for (p, s) in [("2000.50", "0.01"), ("02000", "0.01"), ("2000.", "0.01"), ("2e3", "0.01"), ("2000", "0"), ("-1", "0.01"), ("2000", "0.000000001")] {
        assert!(try_action(&c, hl_order(p, s, NOW), NOW).is_err(), "{p} {s}");
    }
    // Another order type.
    let mut trig = hl_order("2000", "0.01", NOW);
    trig["tif"] = "FrontendMarket".into();
    assert!(try_action(&c, trig, NOW).is_err());
    // Leverage above the grant's 5x.
    let lev = |l: u32| serde_json::json!({ "type": "hl_leverage", "asset": 4, "is_cross": true, "leverage": l, "nonce": NOW });
    assert_eq!(try_action(&c, lev(5), NOW), Ok(()));
    assert_eq!(try_action(&c, lev(6), NOW), Err("leverage above what the grant allows"));
    assert!(try_action(&c, lev(0), NOW).is_err());
    // Cancels are free but still market-bound.
    assert_eq!(try_action(&c, serde_json::json!({ "type": "hl_cancel", "asset": 4, "oid": 1, "nonce": NOW }), NOW), Ok(()));
    assert!(try_action(&c, serde_json::json!({ "type": "hl_cancel", "asset": 0, "oid": 1, "nonce": NOW }), NOW).is_err());
    // Never through the queue.
    let s = c.get_session(wallet(), "agent-1".into()).unwrap();
    let mut s2 = s.clone();
    s2.grant.queue_new_targets = true;
    let a: Action = serde_json::from_value(hl_order("2000", "0.01", NOW)).unwrap();
    assert!(rules::check(&s2, &a, Mode::Queued, NOW, &wallet(), &ext(), &signer()).is_err());
}

#[test]
fn hl_spend_accumulates_across_orders() {
    let mut c = setup();
    exec(&mut c, 1, hl_order("2000", "0.03", NOW + 1000)); // 60 USD
    assert_eq!(c.spent_today(wallet(), "agent-1".into(), rules::HL_CAP_ASSET.into()), U128(60_000_000));
    // 60 + 60 > 100.
    assert_eq!(try_action(&c, hl_order("2000", "0.03", NOW + 2000), NOW + 2000), Err("over the daily cap"));
    assert_eq!(try_action(&c, hl_order("2000", "0.02", NOW + 2000), NOW + 2000), Ok(()));
}

#[test]
fn hl_grant_limits() {
    ctx(owner(), NearToken::ZERO, NOW);
    let mut g = grant();
    g.hl_markets.push(rules::HlMarket { asset: 4, max_leverage: 3 });
    assert!(rules::validate_grant(&g, "k", NOW, &wallet(), &ext(), &signer()).is_err(), "listed twice");
    let mut g = grant();
    g.hl_markets[0].max_leverage = 51;
    assert!(rules::validate_grant(&g, "k", NOW, &wallet(), &ext(), &signer()).is_err());
}

#[test]
fn hl_notional_rounds_up() {
    assert_eq!(hl::notional_micro_usd("3.62695", "0.1").unwrap(), 362_695);
    assert_eq!(hl::notional_micro_usd("1.00000001", "1").unwrap(), 1_000_001);
    assert_eq!(hl::notional_micro_usd("75378", "0.00013").unwrap(), 9_799_140);
}

// ---- Venue arguments: results go only to the account ----

fn venue_grant(c: &mut Extension) {
    let mut g = grant();
    g.evm_venues = vec![
        EvmVenue { chain_id: BASE_SEPOLIA, to: POOL.into(), selector: SUPPLY.into() },
        EvmVenue { chain_id: BASE_SEPOLIA, to: POOL.into(), selector: "0x69328dec".into() },
        EvmVenue { chain_id: BASE_SEPOLIA, to: POOL.into(), selector: "0x573ade81".into() },
        EvmVenue { chain_id: BASE_SEPOLIA, to: ROUTER.into(), selector: "0x04e45aaf".into() },
    ];
    ctx(wallet(), GRANT_DEPOSIT, NOW);
    c.grant_session("agent-1".into(), g);
}

fn withdraw(to: &str) -> serde_json::Value {
    evm(POOL, 0, format!("0x69328dec{}{}{}", word_addr(USDC), word_u(5), word_addr(to)))
}
fn repay(on_behalf: &str) -> serde_json::Value {
    evm(POOL, 0, format!("0x573ade81{}{}{}{}", word_addr(USDC), word_u(5), word_u(2), word_addr(on_behalf)))
}
fn swap(recipient: &str) -> serde_json::Value {
    evm(ROUTER, 0, format!("0x04e45aaf{}{}{}{}{}{}{}", word_addr(USDC), word_addr(WETH), word_u(3000), word_addr(recipient), word_u(5_000_000), word_u(1), word_u(0)))
}

#[test]
fn venue_results_go_to_the_account() {
    let mut c = setup();
    venue_grant(&mut c);
    assert_eq!(try_action(&c, supply(1), NOW), Ok(()));
    assert_eq!(try_action(&c, withdraw(SELF), NOW), Ok(()));
    assert_eq!(try_action(&c, repay(SELF), NOW), Ok(()));
    assert_eq!(try_action(&c, swap(SELF), NOW), Ok(()));
}

#[test]
fn venue_results_to_someone_else_are_refused() {
    let mut c = setup();
    venue_grant(&mut c);
    let other = evm(POOL, 0, format!("{SUPPLY}{}{}{}{}", word_addr(USDC), word_u(1), word_addr(STRANGER), word_u(0)));
    for a in [other, withdraw(STRANGER), repay(STRANGER), swap(STRANGER)] {
        assert_eq!(try_action(&c, a, NOW), Err("venue call sends the result to someone else"));
    }
    // Queued doesn't loosen it.
    let s = c.get_session(wallet(), "agent-1".into()).unwrap();
    let mut s2 = s.clone();
    s2.grant.queue_new_targets = true;
    let a: Action = serde_json::from_value(swap(STRANGER)).unwrap();
    assert_eq!(rules::check(&s2, &a, Mode::Queued, NOW, &wallet(), &ext(), &signer()).err(), Some("venue call sends the result to someone else"));
}

#[test]
fn venue_calls_with_extra_or_dirty_data_are_refused() {
    let mut c = setup();
    venue_grant(&mut c);
    // A trailing word.
    let long = evm(ROUTER, 0, format!("0x04e45aaf{}{}{}{}{}{}{}{}", word_addr(USDC), word_addr(WETH), word_u(3000), word_addr(SELF), word_u(5), word_u(1), word_u(0), word_u(9)));
    assert_eq!(try_action(&c, long, NOW), Err("venue call data has the wrong length"));
    // The recipient word with bits above the address (a token or router
    // reading it differently from us).
    let dirty = format!("0x04e45aaf{}{}{}{}{}{}{}", word_addr(USDC), word_addr(WETH), word_u(3000), format!("01{}", &word_addr(SELF)[2..]), word_u(5), word_u(1), word_u(0));
    assert!(try_action(&c, evm(ROUTER, 0, dirty), NOW).is_err());
}

#[test]
fn venue_calls_need_the_account_named() {
    let mut c = setup();
    let mut g = grant();
    g.evm_self = None;
    ctx(wallet(), GRANT_DEPOSIT, NOW);
    c.grant_session("agent-1".into(), g);
    assert_eq!(try_action(&c, supply(1), NOW), Err("this grant doesn't name the account's EVM address"));
    // And a malformed one is refused at grant time.
    let mut g = grant();
    g.evm_self = Some("0x1234".into());
    assert!(rules::validate_grant(&g, "k", NOW, &wallet(), &ext(), &signer()).is_err());
}

// ---- Payment-only chains ----

/// The same plan as core's reference plans (key 0x11…11, inputs aa…:1 of
/// 6 coins and bb…:0 of 2.5 coins): the extension must produce exactly the
/// signing hashes core does, and core's are checked against bitcoinjs-lib,
/// bitcore-lib-cash and zcash_primitives.
const BTC_SELF: &str = "0014fc7250a211deddc70ee5a2738de5f07817351cef";
const BTC_FRIEND: &str = "0014751e76e8199196d454941c45d1b3a323f1433bd6";
const P2PKH_SELF: &str = "76a914fc7250a211deddc70ee5a2738de5f07817351cef88ac";

fn pay(chain: &str, outs: &[(&str, u64)]) -> serde_json::Value {
    serde_json::json!({ "type": "utxo_pay", "chain": chain,
        "inputs": [{ "txid": "aa".repeat(32), "vout": 1, "value": 600_000_000u64 }, { "txid": "bb".repeat(32), "vout": 0, "value": 250_000_000u64 }],
        "outputs": outs.iter().map(|(s, v)| serde_json::json!({ "script": s, "value": v })).collect::<Vec<_>>(),
        "branch_id": 0x7719_0ad9u32, "expiry_height": 3_900_000u32 })
}

fn utxo_session(c: &mut Extension, daily: u128) {
    let mut g = grant();
    g.utxo = vec![
        UtxoGrant { chain: "bitcoin".into(), self_script: BTC_SELF.into(), recipients: vec![BTC_FRIEND.into()] },
        UtxoGrant { chain: "litecoin".into(), self_script: BTC_SELF.into(), recipients: vec![] },
        UtxoGrant { chain: "bitcoin-cash".into(), self_script: P2PKH_SELF.into(), recipients: vec![] },
        UtxoGrant { chain: "zcash".into(), self_script: P2PKH_SELF.into(), recipients: vec![] },
    ];
    for ch in ["bitcoin", "litecoin", "bitcoin-cash", "zcash"] {
        g.caps.push(Cap { asset: format!("utxo:{ch}"), daily: U128(daily) });
    }
    ctx(wallet(), GRANT_DEPOSIT, NOW);
    c.grant_session("agent-1".into(), g);
}

fn plan_for(c: &Extension, a: serde_json::Value) -> Result<rules::Plan, &'static str> {
    let s = c.get_session(wallet(), "agent-1".into()).unwrap();
    let a: Action = serde_json::from_value(a).unwrap();
    rules::check(&s, &a, Mode::Direct, NOW, &wallet(), &ext(), &signer())
}

#[test]
fn utxo_hashes_match_core() {
    let mut c = setup();
    utxo_session(&mut c, 1_000_000_000);
    for (chain, outs, want) in [
        ("bitcoin", vec![(BTC_FRIEND, 700_000_000u64), (BTC_SELF, 149_999_582)], ["b653c279d489205a266eeb6b874d7f54b4b6e9ab083aa06e29e54a8afd67eec0", "0255fbfcd20169ab418eb0c7a8ac7a91fb578b62004938a347d8f8e8ac77fc2a"]),
        ("litecoin", vec![(BTC_SELF, 700_000_000), (BTC_SELF, 149_999_582)], ["8ed8d4d4f9c4be203275804cb46a12bfbb78226fe28035e0401f97b69f481115", "fd4ad1476ca1cd9adccf06a2b5f6902bb32773141123950ad17409623b645e86"]),
        ("bitcoin-cash", vec![(P2PKH_SELF, 700_000_000), (P2PKH_SELF, 149_999_626)], ["f5439a5e51b666e4354f1b62272b9d16e50fa67f6fbe67d5152ed37de06ca8ae", "fd774a2b242a06e9222bcd1ba546475634eb30dfa3dc62ee1fa9c076988014fa"]),
        ("zcash", vec![(P2PKH_SELF, 700_000_000), (P2PKH_SELF, 149_990_000)], ["d94586719b3fdf4a278657190ffec949bd76d1546529d9d462992639539f4e52", "97ac0fd59bc4416181ec6af4e039ce5f2b2c56bae56c91a0441f5188951e4c31"]),
    ] {
        let p = plan_for(&c, pay(chain, &outs)).unwrap();
        let rules::Kind::Utxo { path, hashes } = p.kind else { panic!("{chain}") };
        assert_eq!(path, chain);
        assert_eq!(hashes.iter().map(hex::encode).collect::<Vec<_>>(), want, "{chain}");
    }
}

#[test]
fn utxo_spend_counts_payment_and_fee() {
    let mut c = setup();
    utxo_session(&mut c, 1_000_000_000);
    // 8.5 in, 7 to the friend, 1.49999582 back: 7.00000418 leaves, fee included.
    let p = plan_for(&c, pay("bitcoin", &[(BTC_FRIEND, 700_000_000), (BTC_SELF, 149_999_582)])).unwrap();
    assert_eq!(p.spends, vec![("utxo:bitcoin".to_string(), 700_000_418)]);
    // A huge fee is a spend too: 8.5 in, 0.1 to the friend, nothing back.
    let p = plan_for(&c, pay("bitcoin", &[(BTC_FRIEND, 10_000_000)])).unwrap();
    assert_eq!(p.spends, vec![("utxo:bitcoin".to_string(), 850_000_000)]);
}

#[test]
fn utxo_attacks_are_refused() {
    let mut c = setup();
    utxo_session(&mut c, 800_000_000);
    let stranger = "0014000000000000000000000000000000000000dead";
    assert_eq!(plan_for(&c, pay("bitcoin", &[(stranger, 1_000)])).err(), Some("recipient not allowed"));
    // Over the cap through the fee alone: 8.5 coins leave, cap is 8.
    assert_eq!(plan_for(&c, pay("bitcoin", &[(BTC_FRIEND, 10_000_000)])).err(), Some("over the daily cap"));
    // More out than in.
    assert_eq!(plan_for(&c, pay("bitcoin", &[(BTC_FRIEND, 900_000_000)])).err(), Some("outputs exceed inputs"));
    // A chain the grant doesn't list, and Dogecoin at all.
    let mut g = grant();
    g.utxo = vec![UtxoGrant { chain: "dogecoin".into(), self_script: P2PKH_SELF.into(), recipients: vec![] }];
    assert!(rules::validate_grant(&g, "k", NOW, &wallet(), &ext(), &signer()).is_err());
    let mut a = pay("bitcoin", &[(BTC_FRIEND, 1_000)]);
    a["chain"] = "dogecoin".into();
    assert_eq!(plan_for(&c, a).err(), Some("payments on this chain not allowed"));
    // Too many inputs, a repeated input, a non-standard script.
    let mut a = pay("bitcoin", &[(BTC_FRIEND, 1_000)]);
    a["inputs"].as_array_mut().unwrap().push(serde_json::json!({ "txid": "cc".repeat(32), "vout": 0, "value": 1 }));
    assert!(plan_for(&c, a).is_err());
    let mut a = pay("bitcoin", &[(BTC_FRIEND, 1_000)]);
    a["inputs"][1] = a["inputs"][0].clone();
    assert_eq!(plan_for(&c, a).err(), Some("an input is listed twice"));
    assert!(plan_for(&c, pay("bitcoin", &[("6a0568656c6c6f", 1_000)])).is_err(), "OP_RETURN");
    // Zcash without its branch ID.
    let mut a = pay("zcash", &[(P2PKH_SELF, 1_000)]);
    a["branch_id"] = 0.into();
    assert!(plan_for(&c, a).is_err());
    // A grant whose own script has another chain's shape.
    let mut g = grant();
    g.utxo = vec![UtxoGrant { chain: "bitcoin".into(), self_script: P2PKH_SELF.into(), recipients: vec![] }];
    assert!(rules::validate_grant(&g, "k", NOW, &wallet(), &ext(), &signer()).is_err());
}

#[test]
fn utxo_payment_executes_with_one_signature_per_input() {
    let mut c = setup();
    utxo_session(&mut c, 1_000_000_000);
    exec(&mut c, 1, pay("bitcoin", &[(BTC_FRIEND, 700_000_000), (BTC_SELF, 149_999_582)]));
    assert_eq!(c.spent_today(wallet(), "agent-1".into(), "utxo:bitcoin".into()), U128(700_000_418));
    // The next one goes over: 7.00000418 + 7.00000418 > 10.
    assert_eq!(plan_for(&c, pay("bitcoin", &[(BTC_FRIEND, 700_000_000), (BTC_SELF, 149_999_582)])).err(), Some("over the daily cap"));
}
