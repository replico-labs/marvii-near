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
        evm_nft_floor: U128(0),
        utxo: vec![],
        sol: None,
        xrpl: None,
        stellar: None,
        tron: None,
        ton: None,
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
    // A chain the grant doesn't list, and an unknown chain at all.
    let mut g = grant();
    g.utxo = vec![UtxoGrant { chain: "monero".into(), self_script: P2PKH_SELF.into(), recipients: vec![] }];
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

mod solana {
    use super::*;
    use crate::rules::{SolGrant, SolVenue};
    use crate::sol;

    const ME: &str = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
    const FRIEND: &str = "7VHUFJHWu2CuExkJcJrzhQPJ2oygupTWkL2A2For4BmE";
    const THIEF: &str = "3Kz9QnDWYB8kMHJUZB6YNp6Cbwjcm5JBEsJfpG5Q2Ns1";
    const JITO_PROG: &str = "DPoo15wWDqpPJJtS2MUZ49aRxqz5ZaaJCJP4z8bLuib";
    const JITO_POOL: &str = "JitoY5pcAxWX6iyP2QdFwTznGb8A99PRCUCVVxB46WZ";
    const JITOSOL: &str = "J1tos8mqbhdGcF3pgj4PCKyVjzWSURcpLZU7pPGHxSYi";
    const FAKE_POOL: &str = "4qQeZ5LwSz6HuupUu8jCtgXyW1mYQcNbFAW1sWZp89HL";
    const ANY: &str = "SysvarC1ock11111111111111111111111111111111";
    const BH: &str = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";

    fn sol_grant() -> Grant {
        let mut g = grant();
        g.sol = Some(SolGrant {
            self_address: ME.into(),
            venues: vec![SolVenue { program: JITO_PROG.into(), scope: JITO_POOL.into() }],
            recipients: vec![FRIEND.into()],
            nft_floor: U128(0),
            perps: vec![],
        });
        g.caps.push(Cap { asset: "sol".into(), daily: U128(2_000_000_000) });
        g.caps.push(Cap { asset: format!("spl:{JITOSOL}"), daily: U128(1_000_000_000) });
        g
    }

    fn setup_sol() -> Extension {
        ctx(owner(), NearToken::ZERO, NOW);
        let mut c = Extension::new(owner(), signer(), DELAY);
        ctx(wallet(), GRANT_DEPOSIT, NOW);
        c.grant_session("agent-1".into(), sol_grant());
        c
    }

    fn ata_of(owner: &str, mint: &str, bump: u8) -> String {
        sol::fmt(&sol::ata(&sol::b58(owner).unwrap(), &sol::b58(mint).unwrap(), &sol::b58(sol::TOKEN).unwrap(), bump))
    }

    fn m(k: &str, signer: bool, writable: bool) -> serde_json::Value {
        serde_json::json!({ "pubkey": k, "signer": signer, "writable": writable })
    }

    /// Jito DepositSol: lamports in, JitoSOL to `dest`, referral fee to `referral`.
    fn deposit(pool: &str, dest: &str, referral: &str, lamports: u64) -> serde_json::Value {
        let mut data = vec![14u8];
        data.extend(lamports.to_le_bytes());
        serde_json::json!({ "program": JITO_PROG, "data": hex::encode(data), "accounts": [
            m(pool, false, true), m(ANY, false, false), m(ANY, false, true), m(ME, true, true),
            m(dest, false, true), m(ANY, false, true), m(referral, false, true), m(JITOSOL, false, true),
            m(sol::SYSTEM, false, false), m(sol::TOKEN, false, false)
        ]})
    }

    fn tx(ixs: Vec<serde_json::Value>, atas: Vec<serde_json::Value>) -> serde_json::Value {
        serde_json::json!({ "type": "sol_tx", "recent_blockhash": BH, "instructions": ixs, "atas": atas })
    }
    fn hint(owner: &str, bump: u8) -> serde_json::Value {
        serde_json::json!({ "owner": owner, "mint": JITOSOL, "token_program": sol::TOKEN, "bump": bump })
    }

    fn run(action: serde_json::Value) -> Result<(), &'static str> {
        let c = setup_sol();
        try_action(&c, action, NOW + 1000)
    }

    #[test]
    fn ata_matches_spl() {
        // From @solana/spl-token's getAssociatedTokenAddressSync (bump 254).
        assert_eq!(ata_of(ME, JITOSOL, 254), "AJKRBfFSWpGzpaZLqHir69WwcxPk9urB8JhSayCELPzs");
    }

    #[test]
    fn jito_stake_to_own_account() {
        let own = ata_of(ME, JITOSOL, 254);
        assert_eq!(run(tx(vec![deposit(JITO_POOL, &own, &own, 100_000_000)], vec![hint(ME, 254)])), Ok(()));
    }

    #[test]
    fn jito_stake_output_to_someone_else_refused() {
        let theirs = ata_of(THIEF, JITOSOL, 255);
        assert_eq!(
            run(tx(vec![deposit(JITO_POOL, &theirs, &theirs, 100_000_000)], vec![hint(THIEF, 255)])),
            Err("venue call sends the result to someone else")
        );
    }

    #[test]
    fn jito_referral_to_someone_else_refused() {
        let own = ata_of(ME, JITOSOL, 254);
        let theirs = ata_of(THIEF, JITOSOL, 255);
        assert_eq!(
            run(tx(vec![deposit(JITO_POOL, &own, &theirs, 1)], vec![hint(ME, 254), hint(THIEF, 255)])),
            Err("referral fee must go to the account")
        );
    }

    #[test]
    fn unlisted_pool_refused() {
        let own = ata_of(ME, JITOSOL, 254);
        assert_eq!(run(tx(vec![deposit(FAKE_POOL, &own, &own, 1)], vec![hint(ME, 254)])), Err("not an allowed venue"));
    }

    #[test]
    fn over_sol_cap_refused() {
        let own = ata_of(ME, JITOSOL, 254);
        assert_eq!(run(tx(vec![deposit(JITO_POOL, &own, &own, 2_000_000_000)], vec![hint(ME, 254)])), Err("over the daily cap"));
    }

    fn transfer(to: &str, lamports: u64) -> serde_json::Value {
        let mut data = vec![2u8, 0, 0, 0];
        data.extend(lamports.to_le_bytes());
        serde_json::json!({ "program": sol::SYSTEM, "data": hex::encode(data), "accounts": [m(ME, true, true), m(to, false, true)] })
    }

    #[test]
    fn sol_payments_only_to_listed() {
        assert_eq!(run(tx(vec![transfer(FRIEND, 1000)], vec![])), Ok(()));
        assert_eq!(run(tx(vec![transfer(THIEF, 1000)], vec![])), Err("recipient not allowed"));
    }

    #[test]
    fn unknown_program_refused() {
        let ix = serde_json::json!({ "program": THIEF, "data": "00", "accounts": [m(ME, true, true)] });
        assert_eq!(run(tx(vec![ix], vec![])), Err("no decoder for this Solana program"));
    }

    #[test]
    fn another_signer_refused() {
        let ix = serde_json::json!({ "program": sol::SYSTEM, "data": hex::encode([2u8,0,0,0,1,0,0,0,0,0,0,0]),
            "accounts": [m(THIEF, true, true), m(FRIEND, false, true)] });
        assert_eq!(run(tx(vec![ix], vec![])), Err("only the account may sign"));
    }

    #[test]
    fn other_system_instructions_refused() {
        // Assign (1): hand the account to another program.
        let ix = serde_json::json!({ "program": sol::SYSTEM, "data": hex::encode([1u8,0,0,0]), "accounts": [m(ME, true, true)] });
        assert_eq!(run(tx(vec![ix], vec![])), Err("only SOL transfers on the System program"));
    }

    #[test]
    fn priority_fee_capped_and_counted() {
        let price = |p: u64| { let mut d = vec![3u8]; d.extend(p.to_le_bytes()); serde_json::json!({ "program": sol::COMPUTE_BUDGET, "data": hex::encode(d), "accounts": [] }) };
        assert_eq!(run(tx(vec![price(sol::MAX_CU_PRICE + 1), transfer(FRIEND, 1)], vec![])), Err("priority fee too high"));
        assert_eq!(run(tx(vec![price(1000), transfer(FRIEND, 1)], vec![])), Ok(()));
    }

    #[test]
    fn executes_through_eddsa() {
        let mut c = setup_sol();
        let own = ata_of(ME, JITOSOL, 254);
        exec(&mut c, 1, tx(vec![deposit(JITO_POOL, &own, &own, 100_000_000)], vec![hint(ME, 254)]));
        let s = c.get_session(wallet(), "agent-1".into()).unwrap();
        assert_eq!(s.nonce, 1);
        // 0.1 SOL plus the 5,000-lamport signature fee.
        assert_eq!(s.spent_on("sol", NOW + 1000), 100_005_000);
    }

    #[test]
    fn message_layout() {
        let a: Action = serde_json::from_value(tx(vec![transfer(FRIEND, 7)], vec![])).unwrap();
        let Action::SolTx { recent_blockhash, instructions, .. } = a else { unreachable!() };
        let ixs = sol::parse(&instructions).unwrap();
        let msg = sol::compile(&sol::b58(ME).unwrap(), &ixs, &sol::b58(&recent_blockhash).unwrap()).unwrap();
        // header 1 signer, 0 read-only signed, 1 read-only unsigned; 3 keys.
        assert_eq!(&msg[..4], &[1, 0, 1, 3]);
        assert_eq!(&msg[4..36], &sol::b58(ME).unwrap());
        assert_eq!(&msg[36..68], &sol::b58(FRIEND).unwrap());
        assert_eq!(&msg[68..100], &[0u8; 32]);
    }
}


mod kamino_borrow {
    use super::*;
    use crate::rules::{SolGrant, SolVenue};
    use crate::sol;

    const ME: &str = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
    const THIEF: &str = "3Kz9QnDWYB8kMHJUZB6YNp6Cbwjcm5JBEsJfpG5Q2Ns1";
    const MARKET: &str = "Fcv68kert8VhyqjD1Cb7bCtPocHm6sWcrKkpQ2vwnYDM";
    const USDC: &str = "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU";
    /// From @solana/web3.js findProgramAddressSync with klend-sdk's seeds.
    const OBLIGATION: &str = "DLCPpr2cczzzwSYyjhq611eUoyQMfkPra5sgJmtRV1Yh";
    const USDC_ATA: &str = "HwpBSwuyVKJi7d9kqqNexc54MS9i4BEDKDVDLeUVjZm8";
    const ANY: &str = "SysvarC1ock11111111111111111111111111111111";
    const BH: &str = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";

    fn setup() -> Extension {
        let mut g = grant();
        g.sol = Some(SolGrant { self_address: ME.into(), venues: vec![SolVenue { program: sol::KAMINO.into(), scope: MARKET.into() }], recipients: vec![], nft_floor: U128(0), perps: vec![] });
        g.caps.push(Cap { asset: "sol".into(), daily: U128(100_000_000) });
        g.caps.push(Cap { asset: format!("spl:{USDC}"), daily: U128(5_000_000) });
        g.caps.push(Cap { asset: format!("borrow:solana:{USDC}"), daily: U128(2_000_000) });
        ctx(owner(), NearToken::ZERO, NOW);
        let mut c = Extension::new(owner(), signer(), DELAY);
        ctx(wallet(), GRANT_DEPOSIT, NOW);
        c.grant_session("agent-1".into(), g);
        c
    }
    fn m(k: &str, signer: bool, writable: bool) -> serde_json::Value {
        serde_json::json!({ "pubkey": k, "signer": signer, "writable": writable })
    }
    fn data(name: &str, amount: u64) -> String {
        let mut d = env::sha256(format!("global:{name}").as_bytes())[..8].to_vec();
        d.extend(amount.to_le_bytes());
        hex::encode(d)
    }
    fn borrow(obligation: &str, dest: &str, amount: u64) -> serde_json::Value {
        serde_json::json!({ "program": sol::KAMINO, "data": data("borrow_obligation_liquidity", amount), "accounts": [
            m(ME, true, false), m(obligation, false, true), m(MARKET, false, false), m(ANY, false, false), m(ANY, false, true),
            m(USDC, false, false), m(ANY, false, true), m(ANY, false, true), m(dest, false, true), m(sol::KAMINO, false, false),
            m(sol::TOKEN, false, false), m(sol::INSTRUCTIONS_SYSVAR, false, false)
        ]})
    }
    fn repay(obligation: &str, amount: u64) -> serde_json::Value {
        serde_json::json!({ "program": sol::KAMINO, "data": data("repay_obligation_liquidity", amount), "accounts": [
            m(ME, true, false), m(obligation, false, true), m(MARKET, false, false), m(ANY, false, true), m(USDC, false, false),
            m(ANY, false, true), m(USDC_ATA, false, true), m(sol::TOKEN, false, false), m(sol::INSTRUCTIONS_SYSVAR, false, false)
        ]})
    }
    fn run(ixs: Vec<serde_json::Value>) -> Result<(), &'static str> {
        let a = serde_json::json!({ "type": "sol_tx", "recent_blockhash": BH, "instructions": ixs,
            "atas": [{ "owner": ME, "mint": USDC, "token_program": sol::TOKEN, "bump": 253 }],
            "obligations": [{ "market": MARKET, "bump": 255 }] });
        try_action(&setup(), a, NOW + 1000)
    }

    #[test]
    fn obligation_matches_sdk() {
        let me = sol::b58(ME).unwrap();
        assert_eq!(sol::fmt(&sol::kamino_obligation(&me, &sol::b58(MARKET).unwrap(), 255)), OBLIGATION);
    }

    #[test]
    fn borrow_to_itself_within_cap() {
        run(vec![borrow(OBLIGATION, USDC_ATA, 2_000_000)]).unwrap();
        assert_eq!(run(vec![borrow(OBLIGATION, USDC_ATA, 2_000_001)]), Err("over the daily cap"));
    }

    #[test]
    fn borrow_to_someone_else_refused() {
        let theirs = sol::fmt(&sol::ata(&sol::b58(THIEF).unwrap(), &sol::b58(USDC).unwrap(), &sol::b58(sol::TOKEN).unwrap(), 255));
        assert_eq!(run(vec![borrow(OBLIGATION, &theirs, 1)]), Err("venue call sends the result to someone else"));
    }

    #[test]
    fn repay_only_own_obligation() {
        run(vec![repay(OBLIGATION, 1_000_000)]).unwrap();
        assert_eq!(run(vec![repay(ANY, 1_000_000)]), Err("not the account's own obligation"));
    }
}


mod tensor_nft {
    use super::*;
    use crate::rules::{SolGrant, SolVenue};
    use crate::sol;

    // From @tensor-foundation/marketplace 1.0.0 on devnet (13 Nov 2026): the
    // wallet buying 5Vo6… (listed at 0.01 SOL), and listing DHHN… at 0.05.
    const ME: &str = "8Mq6dzJzfnDS6tCnhkwuehRGQEv9tPM8KsdhSYDa3ArE";
    const BUY_DATA: &str = "447f2b08d41ff972809698000000000001640000";
    const LIST_DATA: &str = "066eff121024081e80f0fa0200000000000000016d57ceefc0c9fb982f041e6c9cfe89879658703fe671e5ce4309e0844180a88b00";
    const BUY: [(&str, u8); 25] = [
            ("t8zkpFfoVkUBVaWT4L3Va9dRg2oCHoicApQjejX1L9o", 1),
            ("8Mq6dzJzfnDS6tCnhkwuehRGQEv9tPM8KsdhSYDa3ArE", 0),
            ("DYVyPcsgVLmNmqkjKyKopAYBUByQ9DKewFZgfcq5qhvY", 1),
            ("5caroPhjRzVm99zT32aKs3dwE3AXyoboWeN9sF7rSwHu", 1),
            ("6mwMgbVAboxaSW7a845wPo7j6Uf6ydgzd9AzNG2bPGJQ", 1),
            ("5Vo6HsnaNX12oSDSeZL8jC5HpFZUZA69C1p1DuvgsRiw", 0),
            ("AeqRJJ98xwDKoX65cwLQhzsYLEzwLBjAgKuwV8ciiUKB", 1),
            ("8Mq6dzJzfnDS6tCnhkwuehRGQEv9tPM8KsdhSYDa3ArE", 3),
            ("8Mq6dzJzfnDS6tCnhkwuehRGQEv9tPM8KsdhSYDa3ArE", 1),
            ("TCMPhJdwDryooaGtiocG1u3xcYbRpiJzb283XfCZsDp", 0),
            ("AeqRJJ98xwDKoX65cwLQhzsYLEzwLBjAgKuwV8ciiUKB", 1),
            ("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", 0),
            ("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL", 0),
            ("TCMPhJdwDryooaGtiocG1u3xcYbRpiJzb283XfCZsDp", 0),
            ("11111111111111111111111111111111", 0),
            ("8gQpryZbknWhMj26TqmAuoTH6Svxm1ifvcfnxMHA81SB", 1),
            ("2iE2ttsNnaYD65jF4XFSqEPY6vn3AKQMYm1oF5HfbDjA", 0),
            ("8PFKyBcJ1god7vHgVNm14afkQhtsp6B2HqHdi64P8UMV", 1),
            ("B2auV7n8UQVY9Jaqn9QPLKV8W3GNGEmE1PmDfRc8FK1m", 1),
            ("TCMPhJdwDryooaGtiocG1u3xcYbRpiJzb283XfCZsDp", 0),
            ("auth9SigNpDKz4sJJ1DfCTuZrZNSAgh9sFD3rboVmgg", 0),
            ("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s", 0),
            ("Sysvar1nstructions1111111111111111111111111", 0),
            ("TCMPhJdwDryooaGtiocG1u3xcYbRpiJzb283XfCZsDp", 0),
            ("AeqRJJ98xwDKoX65cwLQhzsYLEzwLBjAgKuwV8ciiUKB", 1)
    ];
    const LIST: [(&str, u8); 19] = [
            ("8Mq6dzJzfnDS6tCnhkwuehRGQEv9tPM8KsdhSYDa3ArE", 2),
            ("5K2srr9idQ6JrWuujkB7orhonsFUSPve6CLHM2p1AxW1", 1),
            ("AR3eUgu6HQezD61UzdKHYRdrVeVkGeCe36va6omZEwzA", 1),
            ("4MtwkwDBnV8APDSxgmBmkAAgmqM6UyUDKXVW3q5dpN99", 1),
            ("DHHNeX2dpJB2XFVBycnjESexFkeczUTHQBBug3eCMjtD", 0),
            ("8Mq6dzJzfnDS6tCnhkwuehRGQEv9tPM8KsdhSYDa3ArE", 3),
            ("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", 0),
            ("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL", 0),
            ("TCMPhJdwDryooaGtiocG1u3xcYbRpiJzb283XfCZsDp", 0),
            ("11111111111111111111111111111111", 0),
            ("uxDXRXRBSouYxDJF9Ut7jD2B8qyGg3CAfU7yfS1L6j6", 1),
            ("Gxn9LvBQFCrpfoPjLoj39UApjGEoJKwrzoiZRLb61BMQ", 0),
            ("EBqW4oWAeKqZf6vEoRUCJtw5s7JVHmjYcgr7mm4B6ZmN", 1),
            ("JCJn11sgsUqF9LbrPdycCfVyDT7JWNnC15jhwCEvhXsB", 1),
            ("TCMPhJdwDryooaGtiocG1u3xcYbRpiJzb283XfCZsDp", 0),
            ("auth9SigNpDKz4sJJ1DfCTuZrZNSAgh9sFD3rboVmgg", 0),
            ("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s", 0),
            ("Sysvar1nstructions1111111111111111111111111", 0),
            ("TCMPhJdwDryooaGtiocG1u3xcYbRpiJzb283XfCZsDp", 0)
    ];
    const BH: &str = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";

    fn setup(floor: u128) -> Extension {
        let mut g = grant();
        g.sol = Some(SolGrant { self_address: ME.into(), venues: vec![SolVenue { program: sol::TENSOR.into(), scope: sol::TENSOR.into() }], recipients: vec![], nft_floor: U128(floor), perps: vec![] });
        g.caps.push(Cap { asset: "sol".into(), daily: U128(100_000_000) });
        g.caps.push(Cap { asset: "nft:solana".into(), daily: U128(2) });
        ctx(owner(), NearToken::ZERO, NOW);
        let mut c = Extension::new(owner(), signer(), DELAY);
        ctx(wallet(), GRANT_DEPOSIT, NOW);
        c.grant_session("agent-1".into(), g);
        c
    }
    fn ix(data: &str, accts: &[(&str, u8)]) -> serde_json::Value {
        let a: Vec<_> = accts.iter().map(|(k, r)| serde_json::json!({ "pubkey": k, "signer": r & 2 != 0, "writable": r & 1 != 0 })).collect();
        serde_json::json!({ "program": sol::TENSOR, "data": data, "accounts": a })
    }
    /// The ATA bumps (findProgramAddressSync) for the two mints.
    fn run(c: &Extension, i: serde_json::Value, mint: &str, bump: u8) -> Result<(), &'static str> {
        let a = serde_json::json!({ "type": "sol_tx", "recent_blockhash": BH, "instructions": [i],
            "atas": [{ "owner": ME, "mint": mint, "token_program": sol::TOKEN, "bump": bump }] });
        try_action(c, a, NOW + 1000)
    }
    fn bump_for(mint: &str, ata: &str) -> u8 {
        (0..=255u8).rev().find(|b| sol::fmt(&sol::ata(&sol::b58(ME).unwrap(), &sol::b58(mint).unwrap(), &sol::b58(sol::TOKEN).unwrap(), *b)) == ata).expect("bump")
    }

    #[test]
    fn buy_into_own_account() {
        let b = bump_for(BUY[5].0, BUY[2].0);
        run(&setup(0), ix(BUY_DATA, &BUY), BUY[5].0, b).unwrap();
        // Delivered to someone else's token account: refused.
        let mut other = BUY;
        other[1] = ("AeqRJJ98xwDKoX65cwLQhzsYLEzwLBjAgKuwV8ciiUKB", 0);
        assert_eq!(run(&setup(0), ix(BUY_DATA, &other), BUY[5].0, b), Err("venue call sends the result to someone else"));
    }

    #[test]
    fn buy_counts_price_fees_and_royalties() {
        // 0.01 SOL, 3% fees, 100% royalty share, a token account, the fee:
        // with a 0.03 SOL cap that fits; at 0.015 it doesn't.
        let b = bump_for(BUY[5].0, BUY[2].0);
        let c = setup(0);
        run(&c, ix(BUY_DATA, &BUY), BUY[5].0, b).unwrap();
        let mut g = grant();
        g.sol = Some(SolGrant { self_address: ME.into(), venues: vec![SolVenue { program: sol::TENSOR.into(), scope: sol::TENSOR.into() }], recipients: vec![], nft_floor: U128(0), perps: vec![] });
        g.caps.push(Cap { asset: "sol".into(), daily: U128(15_000_000) });
        ctx(owner(), NearToken::ZERO, NOW);
        let mut c = Extension::new(owner(), signer(), DELAY);
        ctx(wallet(), GRANT_DEPOSIT, NOW);
        c.grant_session("agent-1".into(), g);
        assert_eq!(run(&c, ix(BUY_DATA, &BUY), BUY[5].0, b), Err("over the daily cap"));
    }

    #[test]
    fn list_needs_floor() {
        let b = bump_for(LIST[4].0, LIST[1].0);
        run(&setup(50_000_000), ix(LIST_DATA, &LIST), LIST[4].0, b).unwrap();
        assert_eq!(run(&setup(50_000_001), ix(LIST_DATA, &LIST), LIST[4].0, b), Err("listing price below the floor"));
        assert_eq!(run(&setup(0), ix(LIST_DATA, &LIST), LIST[4].0, b), Err("listing price below the floor"));
    }

    #[test]
    fn list_to_private_buyer_refused() {
        let b = bump_for(LIST[4].0, LIST[1].0);
        // private_taker = Some(the seller): bytes 18.. become 01 + key.
        let mut d = hex::decode(LIST_DATA).unwrap();
        let taker = sol::b58("AeqRJJ98xwDKoX65cwLQhzsYLEzwLBjAgKuwV8ciiUKB").unwrap();
        d.splice(18..19, [vec![1u8], taker.to_vec()].concat());
        assert_eq!(run(&setup(1), ix(&hex::encode(d), &LIST), LIST[4].0, b), Err("no private buyer"));
    }
}

mod xrp_ledger {
    use super::*;
    use crate::rules::XrplGrant;
    use crate::xrpl::{self, XrplTx};

    const ME: &str = "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh";
    const PK: &str = "0330E7FC9D56BB25D6893BA3F317AE5BCF33B3291BD63DB32654A313222F7FD020";
    const ISS: &str = "rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe";

    fn xg() -> XrplGrant {
        XrplGrant {
            self_address: ME.into(), public_key: PK.into(), assets: vec![format!("USD.{ISS}")],
            recipients: vec![ISS.into()], dex: true, amm: true, nft: true,
        }
    }
    fn tx(op: serde_json::Value) -> XrplTx {
        serde_json::from_value(serde_json::json!({ "sequence": 7, "fee": "12", "last_ledger_sequence": 1000, "op": op })).unwrap()
    }
    fn usd(v: &str) -> serde_json::Value { serde_json::json!({ "currency": "USD", "issuer": ISS, "value": v }) }
    fn xrp(drops: &str) -> serde_json::Value { serde_json::json!({ "currency": "XRP", "value": drops }) }
    fn enc(op: serde_json::Value) -> String {
        ctx(owner(), NearToken::ZERO, NOW);
        hex::encode_upper(xrpl::check(&xg(), &tx(op), false).unwrap().0)
    }

    // Expected bytes from ripple-binary-codec 2's encodeForSigning (prefix removed).
    #[test]
    fn offer_matches_codec() {
        assert_eq!(enc(serde_json::json!({ "kind": "offer_create", "taker_pays": usd("1.5"), "taker_gets": xrp("2000000"), "flags": 0x80000 })),
            "12000722000800002400000007201B000003E864D485543DF729C0000000000000000000000000005553440000000000F667B0CA50CC7709A220B0561B85E53A48461FA86540000000001E848068400000000000000C73210330E7FC9D56BB25D6893BA3F317AE5BCF33B3291BD63DB32654A313222F7FD0208114B5F762798A53D543A014CAF8B297CFF8F2F937E8");
    }
    #[test]
    fn trust_matches_codec() {
        assert_eq!(enc(serde_json::json!({ "kind": "trust_set", "limit": usd("1000") })),
            "12001422000200002400000007201B000003E863D5438D7EA4C680000000000000000000000000005553440000000000F667B0CA50CC7709A220B0561B85E53A48461FA868400000000000000C73210330E7FC9D56BB25D6893BA3F317AE5BCF33B3291BD63DB32654A313222F7FD0208114B5F762798A53D543A014CAF8B297CFF8F2F937E8");
    }
    #[test]
    fn amm_matches_codec() {
        assert_eq!(enc(serde_json::json!({ "kind": "amm_deposit", "asset": { "currency": "XRP" }, "asset2": { "currency": "USD", "issuer": ISS }, "amount": xrp("1000000"), "amount2": usd("0.25") })),
            "12002422001000002400000007201B000003E86140000000000F424068400000000000000C6BD448E1BC9BF040000000000000000000000000005553440000000000F667B0CA50CC7709A220B0561B85E53A48461FA873210330E7FC9D56BB25D6893BA3F317AE5BCF33B3291BD63DB32654A313222F7FD0208114B5F762798A53D543A014CAF8B297CFF8F2F937E80318000000000000000000000000000000000000000004180000000000000000000000005553440000000000F667B0CA50CC7709A220B0561B85E53A48461FA8");
        assert_eq!(enc(serde_json::json!({ "kind": "amm_withdraw_all", "asset": { "currency": "XRP" }, "asset2": { "currency": "USD", "issuer": ISS } })),
            "12002522000200002400000007201B000003E868400000000000000C73210330E7FC9D56BB25D6893BA3F317AE5BCF33B3291BD63DB32654A313222F7FD0208114B5F762798A53D543A014CAF8B297CFF8F2F937E80318000000000000000000000000000000000000000004180000000000000000000000005553440000000000F667B0CA50CC7709A220B0561B85E53A48461FA8");
    }
    #[test]
    fn nft_matches_codec() {
        assert_eq!(enc(serde_json::json!({ "kind": "nft_mint", "taxon": 0, "uri": "697066733A2F2F62616679" })),
            "12001922000000082400000007201B000003E8202A0000000068400000000000000C73210330E7FC9D56BB25D6893BA3F317AE5BCF33B3291BD63DB32654A313222F7FD020750B697066733A2F2F626166798114B5F762798A53D543A014CAF8B297CFF8F2F937E8");
        assert_eq!(enc(serde_json::json!({ "kind": "nft_buy_offer", "nftoken_id": "000800006203F49C21D5D6E022CB16DE3538F248662FC73C00000099B00000A8", "owner": ISS, "amount": xrp("5000000") })),
            "12001B22000000002400000007201B000003E85A000800006203F49C21D5D6E022CB16DE3538F248662FC73C00000099B00000A86140000000004C4B4068400000000000000C73210330E7FC9D56BB25D6893BA3F317AE5BCF33B3291BD63DB32654A313222F7FD0208114B5F762798A53D543A014CAF8B297CFF8F2F937E88214F667B0CA50CC7709A220B0561B85E53A48461FA8");
    }
    #[test]
    fn nft_cancel_matches_codec() {
        assert_eq!(enc(serde_json::json!({ "kind": "nft_cancel_offers", "offers": ["F7848C8B999C645C0ED49AAB5D75AF324A173AB087DC6D63DC0EE51555CA68EC"] })),
            "12001C22000000002400000007201B000003E868400000000000000C73210330E7FC9D56BB25D6893BA3F317AE5BCF33B3291BD63DB32654A313222F7FD0208114B5F762798A53D543A014CAF8B297CFF8F2F937E8041320F7848C8B999C645C0ED49AAB5D75AF324A173AB087DC6D63DC0EE51555CA68EC");
    }
    #[test]
    fn payment_matches_codec() {
        assert_eq!(enc(serde_json::json!({ "kind": "payment", "destination": ISS, "amount": usd("12.3456"), "destination_tag": 42 })),
            "120000220000000024000000072E0000002A201B000003E861D4C462D3664100000000000000000000000000005553440000000000F667B0CA50CC7709A220B0561B85E53A48461FA868400000000000000C73210330E7FC9D56BB25D6893BA3F317AE5BCF33B3291BD63DB32654A313222F7FD0208114B5F762798A53D543A014CAF8B297CFF8F2F937E88314F667B0CA50CC7709A220B0561B85E53A48461FA8");
    }

    fn check(op: serde_json::Value) -> Result<Vec<(String, u128)>, &'static str> {
        ctx(owner(), NearToken::ZERO, NOW);
        xrpl::check(&xg(), &tx(op), false).map(|r| r.2)
    }

    #[test]
    fn refusals() {
        let other = "rLHzPsX6oXkzU2qL12kHCH8G8cnZv1rBJh";
        assert_eq!(check(serde_json::json!({ "kind": "payment", "destination": other, "amount": xrp("1") })), Err("recipient not allowed"));
        let eur = serde_json::json!({ "currency": "EUR", "issuer": ISS, "value": "1" });
        assert_eq!(check(serde_json::json!({ "kind": "offer_create", "taker_pays": eur, "taker_gets": xrp("1") })), Err("asset not allowed"));
        // tfPartialPayment-style or unknown flags never pass.
        assert_eq!(check(serde_json::json!({ "kind": "offer_create", "taker_pays": usd("1"), "taker_gets": xrp("1"), "flags": 0x1 })), Err("offer flag not allowed"));
        assert_eq!(check(serde_json::json!({ "kind": "payment", "destination": ISS, "amount": usd("0.0000001") })), Err("more than 6 decimals"));
    }

    #[test]
    fn spends_counted() {
        // An offer spends what it gives, plus the fee.
        assert_eq!(check(serde_json::json!({ "kind": "offer_create", "taker_pays": xrp("2000000"), "taker_gets": usd("1.5") })),
            Ok(vec![("xrp".into(), 12), (format!("xrpl:USD.{ISS}"), 1_500_000)]));
        assert_eq!(check(serde_json::json!({ "kind": "amm_withdraw_all", "asset": { "currency": "XRP" }, "asset2": { "currency": "USD", "issuer": ISS } })),
            Ok(vec![("xrp".into(), 12)]));
    }
}

mod stellar_ledger {
    use super::*;
    use crate::rules::StellarGrant;
    use crate::stellar::{self, StellarTx};

    const ME: &str = "GCFIRY65OQE7DFP5KLNS2PF2LVZMUZYJX4OZIEQ36N2IQANUB5XVYOJR";
    const ISS: &str = "GCATS5YOVB6ROX2WUNKGNQ2MP3GMXDMKSG2O4N5CLX3A6W4PZGZZI55U";

    fn sg() -> StellarGrant {
        StellarGrant { self_address: ME.into(), assets: vec![format!("USD:{ISS}"), format!("MARVIITOKEN1:{ISS}")], blend: vec![], recipients: vec![ISS.into()], dex: true, amm: true }
    }
    fn usd() -> serde_json::Value { serde_json::json!({ "code": "USD", "issuer": ISS }) }
    fn xlm() -> serde_json::Value { serde_json::json!({ "code": "XLM" }) }
    fn tx(ops: serde_json::Value, fee: u32) -> StellarTx {
        serde_json::from_value(serde_json::json!({ "seq_num": "42", "fee": fee, "max_time": "1800000000", "ops": ops })).unwrap()
    }

    /// The same eight operations built with @stellar/stellar-sdk 13: XDR and hash match.
    #[test]
    fn matches_sdk() {
        ctx(owner(), NearToken::ZERO, NOW);
        let ops = serde_json::json!([
            { "kind": "payment", "destination": ISS, "asset": usd(), "amount": "15000000" },
            { "kind": "change_trust", "asset": usd(), "limit": "10000000000" },
            { "kind": "sell_offer", "selling": usd(), "buying": xlm(), "amount": "20000000", "price_n": 3, "price_d": 2 },
            { "kind": "swap_send", "send_asset": xlm(), "send_amount": "100000000", "dest_asset": usd(), "dest_min": "5000000" },
            { "kind": "pool_trust", "asset_a": xlm(), "asset_b": usd() },
            { "kind": "pool_deposit", "asset_a": xlm(), "asset_b": usd(), "max_a": "100000000", "max_b": "50000000", "min_price": [1, 3], "max_price": [3, 1] },
            { "kind": "pool_withdraw", "asset_a": xlm(), "asset_b": usd(), "amount": "10000000", "min_a": "0", "min_b": "0" },
            { "kind": "payment", "destination": ISS, "asset": { "code": "MARVIITOKEN1", "issuer": ISS }, "amount": "1" }
        ]);
        let (xdr, hash, spends) = stellar::check(&sg(), &tx(ops, 800), false, stellar::TESTNET).unwrap();
        assert_eq!(hex::encode(&xdr), "000000008a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c00000320000000000000002a000000010000000000000000000000006b49d20000000000000000080000000000000001000000008139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b3940000000155534400000000008139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b3940000000000e4e1c000000000000000060000000155534400000000008139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b39400000002540be40000000000000000030000000155534400000000008139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b394000000000000000001312d0000000003000000020000000000000000000000000000000d000000000000000005f5e100000000008a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c0000000155534400000000008139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b39400000000004c4b400000000000000000000000060000000300000000000000000000000155534400000000008139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b3940000001e7fffffffffffffff0000000000000016274f2c9a53bb2448e00e66e073fd8a22659e552808960a87505305079520aaee0000000005f5e1000000000002faf080000000010000000300000003000000010000000000000017274f2c9a53bb2448e00e66e073fd8a22659e552808960a87505305079520aaee0000000000989680000000000000000000000000000000000000000000000001000000008139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b394000000024d4152564949544f4b454e31000000008139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b394000000000000000100000000");
        assert_eq!(hex::encode(hash), "2652f16dd93cca3fc527bf49a7fe34d9f416817fa2a026885879b7ea749cff51");
        assert_eq!(spends[0], ("xlm".to_string(), 800 + 100_000_000 + 100_000_000));
    }

    #[test]
    fn refusals() {
        ctx(owner(), NearToken::ZERO, NOW);
        let stranger = "GBRPYHIL2CI3FNQ4BXLFMNDLFJUNPU2HY3ZMFSHONUCEOASW7QC7OX2H";
        let pay = serde_json::json!([{ "kind": "payment", "destination": stranger, "asset": xlm(), "amount": "1" }]);
        assert_eq!(stellar::check(&sg(), &tx(pay, 100), false, stellar::TESTNET).map(|_| ()), Err("recipient not allowed"));
        let eur = serde_json::json!([{ "kind": "sell_offer", "selling": { "code": "EUR", "issuer": ISS }, "buying": xlm(), "amount": "1", "price_n": 1, "price_d": 1 }]);
        assert_eq!(stellar::check(&sg(), &tx(eur, 100), false, stellar::TESTNET).map(|_| ()), Err("asset not allowed"));
        let fee = serde_json::json!([{ "kind": "change_trust", "asset": usd(), "limit": "1" }]);
        assert_eq!(stellar::check(&sg(), &tx(fee, stellar::MAX_FEE + 1), false, stellar::TESTNET).map(|_| ()), Err("fee outside 1 stroop to 0.01 XLM"));
    }
}

mod aave_borrow {
    use super::*;

    fn borrow(asset: &str, amount: u128, mode: u128, on_behalf: &str) -> serde_json::Value {
        evm(POOL, 0, format!("0xa415bcad{}{}{}{}{}", word_addr(asset), word_u(amount), word_u(mode), word_u(0), word_addr(on_behalf)))
    }

    fn with_borrow() -> Extension {
        ctx(owner(), NearToken::ZERO, NOW);
        let mut c = Extension::new(owner(), signer(), DELAY);
        let mut g = grant();
        g.evm_venues.push(EvmVenue { chain_id: BASE_SEPOLIA, to: POOL.into(), selector: "0xa415bcad".into() });
        g.caps.push(Cap { asset: format!("borrow:evm:{BASE_SEPOLIA}:{USDC}"), daily: U128(10_000_000) });
        ctx(wallet(), GRANT_DEPOSIT, NOW);
        c.grant_session("agent-1".into(), g);
        c
    }

    #[test]
    fn borrow_for_itself_within_cap() {
        let c = with_borrow();
        assert_eq!(try_action(&c, borrow(USDC, 5_000_000, 2, SELF), NOW + 1000), Ok(()));
        assert_eq!(try_action(&c, borrow(USDC, 20_000_000, 2, SELF), NOW + 1000), Err("over the daily cap"));
    }

    #[test]
    fn borrow_for_someone_else_refused() {
        let c = with_borrow();
        assert_eq!(try_action(&c, borrow(USDC, 1, 2, STRANGER), NOW + 1000), Err("venue call sends the result to someone else"));
    }

    #[test]
    fn stable_rate_refused() {
        let c = with_borrow();
        assert_eq!(try_action(&c, borrow(USDC, 1, 1, SELF), NOW + 1000), Err("borrow at the variable rate only"));
    }
}


mod lido {
    use super::*;
    // The addresses only need to be venues here; the real Hoodi ones are in the agent.
    const WQ: &str = "0xfe56573178f1bcdf53f01a6e9977670dcbbd9186";
    const STETH: &str = "0x3508a952176b3c15387c97be809eaffb1982176a";

    fn setup() -> Extension {
        ctx(owner(), NearToken::ZERO, NOW);
        let mut c = Extension::new(owner(), signer(), DELAY);
        let mut g = grant();
        g.queue_new_targets = false;
        g.evm_venues.push(EvmVenue { chain_id: BASE_SEPOLIA, to: STETH.into(), selector: "0xa1903eab".into() });
        g.evm_venues.push(EvmVenue { chain_id: BASE_SEPOLIA, to: WQ.into(), selector: "0xd6681042".into() });
        g.caps.push(Cap { asset: format!("evm:{BASE_SEPOLIA}"), daily: U128(10u128.pow(17)) });
        ctx(wallet(), GRANT_DEPOSIT, NOW);
        c.grant_session("agent-1".into(), g);
        c
    }
    fn request(amounts: &[u128], owner: &str) -> serde_json::Value {
        let mut d = format!("0xd6681042{}{}{}", word_u(0x40), word_addr(owner), word_u(amounts.len() as u128));
        for a in amounts {
            d += &word_u(*a);
        }
        evm(WQ, 0, d)
    }

    #[test]
    fn stake_counts_the_eth() {
        let c = setup();
        let submit = |v: u128| evm(STETH, v, format!("0xa1903eab{}", word_addr(STRANGER)));
        assert_eq!(try_action(&c, submit(5 * 10u128.pow(16)), NOW + 1000), Ok(()));
        assert_eq!(try_action(&c, submit(2 * 10u128.pow(17)), NOW + 1000), Err("over the daily cap"));
    }

    #[test]
    fn withdrawal_request_only_for_itself() {
        let c = setup();
        assert_eq!(try_action(&c, request(&[10u128.pow(16)], SELF), NOW + 1000), Ok(()));
        assert_eq!(try_action(&c, request(&[10u128.pow(16)], STRANGER), NOW + 1000), Err("venue call sends the result to someone else"));
    }

    #[test]
    fn withdrawal_request_strict_encoding() {
        let c = setup();
        // An offset pointing elsewhere could make the contract read another owner.
        let bad = evm(WQ, 0, format!("0xd6681042{}{}{}{}{}", word_u(0x60), word_addr(SELF), word_u(0), word_u(1), word_u(1)));
        assert_eq!(try_action(&c, bad, NOW + 1000), Err("venue call data has the wrong length"));
    }
}


mod seaport_nft {
    use super::*;
    use crate::seaport;

    const SEAPORT: &str = "0x0000000000000068f116a894984e2db1123eb395";
    const ME: &str = "0x48f0deef2c7e15ea9b4398a6875b43b5ab6ea81e";
    const NFT: &str = "0x1111111111111111111111111111111111111111";
    const FEE: &str = "0x2222222222222222222222222222222222222222";
    const Z: &str = "0x0000000000000000000000000000000000000000";
    const Z32: &str = "0x0000000000000000000000000000000000000000000000000000000000000000";

    fn order(price: u128, fee: u128) -> serde_json::Value {
        serde_json::json!({
            "chain_id": BASE_SEPOLIA, "seaport": SEAPORT, "offerer": ME, "zone": Z,
            "offer": [{ "item_type": 2, "token": NFT, "identifier": "115792089237316195423570985008687907853269984665640564039457584007913129639935", "start_amount": "1", "end_amount": "1" }],
            "consideration": [
                { "item_type": 0, "token": Z, "identifier": "0", "start_amount": price.to_string(), "end_amount": price.to_string(), "recipient": ME },
                { "item_type": 0, "token": Z, "identifier": "0", "start_amount": fee.to_string(), "end_amount": fee.to_string(), "recipient": FEE }
            ],
            "order_type": 0, "start_time": 1763000000u64, "end_time": 1765000000u64, "zone_hash": Z32,
            "salt": "12345678901234567890123456789", "conduit_key": Z32, "counter": "0"
        })
    }
    fn setup(floor: u128) -> Extension {
        ctx(owner(), NearToken::ZERO, NOW);
        let mut c = Extension::new(owner(), signer(), DELAY);
        let mut g = grant();
        g.queue_new_targets = false;
        g.evm_self = Some(ME.into());
        g.evm_nft_floor = U128(floor);
        g.evm_venues.push(EvmVenue { chain_id: BASE_SEPOLIA, to: SEAPORT.into(), selector: "0xfb0f3ee1".into() });
        g.evm_venues.push(EvmVenue { chain_id: BASE_SEPOLIA, to: NFT.into(), selector: "0xa22cb465".into() });
        g.caps.push(Cap { asset: format!("nft:evm:{BASE_SEPOLIA}"), daily: U128(2) });
        g.caps.push(Cap { asset: format!("evm:{BASE_SEPOLIA}"), daily: U128(10u128.pow(17)) });
        ctx(wallet(), GRANT_DEPOSIT, NOW);
        c.grant_session("agent-1".into(), g);
        c
    }
    fn list(c: &Extension, o: serde_json::Value) -> Result<(), &'static str> {
        try_action(c, serde_json::json!({ "type": "seaport_order", "order": o }), NOW + 1000)
    }

    #[test]
    fn hash_matches_viem_and_seaport() {
        // The domain separator Seaport 1.6's information() returns on Base Sepolia.
        assert_eq!(hex::encode(seaport::domain_separator(84532, &evm::parse_address(SEAPORT).unwrap())), "4778aa25cd18fc3b8e9d053bb369734ae2ce8511b24dd9b00010466bdb496b45");
        // viem hashTypedData; Seaport's getOrderHash gave the struct hash inside it.
        let o: seaport::SeaportOrder = serde_json::from_value(order(9_000_000_000_000_000, 900_000_000_000_000)).unwrap();
        assert_eq!(hex::encode(seaport::order_hash(&o).unwrap()), "ba62f72a26235c28f0150b047268a12b7cf17e1f2ce045ac12ac394c1fc28de2");
    }

    #[test]
    fn listing_rules() {
        let c = setup(9_000_000_000_000_000);
        list(&c, order(9_000_000_000_000_000, 900_000_000_000_000)).unwrap();
        assert_eq!(list(&c, order(8_999_999_999_999_999, 0)), Err("listing price below the floor"));
        assert_eq!(list(&c, order(9_000_000_000_000_000, 900_000_000_000_001)), Err("fees over a tenth of the price"));
        let mut o = order(9_000_000_000_000_000, 0);
        o["offerer"] = FEE.into();
        assert_eq!(list(&c, o), Err("the account must be the offerer"));
        let mut o = order(9_000_000_000_000_000, 0);
        o["conduit_key"] = "0x0000007b02230091a7ed01230072f7006a004d60a8d4e71d599b8104250f0000".into();
        assert_eq!(list(&c, o), Err("no conduit: approve Seaport itself"));
        assert_eq!(list(&setup(0), order(9_000_000_000_000_000, 0)), Err("listing price below the floor"));
    }

    #[test]
    fn approval_only_to_a_venue() {
        let c = setup(1);
        let approve_all = |op: &str| evm(NFT, 0, format!("0xa22cb465{}{}", word_addr(op), word_u(1)));
        assert_eq!(try_action(&c, approve_all(SEAPORT), NOW + 1000), Ok(()));
        assert_eq!(try_action(&c, approve_all(STRANGER), NOW + 1000), Err("operator is not a venue"));
    }

    #[test]
    fn basic_order_buys_only() {
        let c = setup(1);
        let basic = |order_type: u128| {
            let mut d = format!("0xfb0f3ee1{}", word_u(0x20));
            for w in 0..18 {
                d += &word_u(if w == 8 { order_type } else { 0 });
            }
            evm(SEAPORT, 10u128.pow(16), d)
        };
        assert_eq!(try_action(&c, basic(0), NOW + 1000), Ok(()));
        // ERC721_TO_ERC20 (16): accepting an offer gives the NFT away.
        assert_eq!(try_action(&c, basic(16), NOW + 1000), Err("only buying an NFT through Seaport"));
    }
}


mod ref_swap {
    use super::*;

    fn ref_id() -> AccountId { "ref-finance-101.testnet".parse().unwrap() }
    fn setup() -> Extension {
        ctx(owner(), NearToken::ZERO, NOW);
        let mut c = Extension::new(owner(), signer(), DELAY);
        let mut g = grant();
        g.near_venues.push(NearVenue { receiver_id: ref_id(), method: "pool:1084".into() });
        ctx(wallet(), GRANT_DEPOSIT, NOW);
        c.grant_session("agent-1".into(), g);
        c
    }
    fn send(to: &str, msg: &str) -> serde_json::Value {
        let args = serde_json::json!({ "receiver_id": to, "amount": "10", "msg": msg }).to_string();
        serde_json::json!({ "type": "near_call", "receiver_id": token(), "method": "ft_transfer_call", "args": args, "deposit": "1" })
    }
    fn swap(pool: u64, extra: Option<(&str, &str)>, min: &str) -> String {
        let mut a = serde_json::json!({ "pool_id": pool, "token_in": token(), "token_out": "usdt.itachicara.testnet", "amount_in": "10", "min_amount_out": min });
        if let Some((k, v)) = extra {
            a[k] = v.into();
        }
        serde_json::json!({ "actions": [a] }).to_string()
    }

    #[test]
    fn listed_pool_swaps() {
        let c = setup();
        assert_eq!(try_action(&c, send("ref-finance-101.testnet", &swap(1084, None, "1")), NOW + 1000), Ok(()));
        // A plain deposit (empty msg) too.
        assert_eq!(try_action(&c, send("ref-finance-101.testnet", ""), NOW + 1000), Ok(()));
    }

    #[test]
    fn refusals() {
        let c = setup();
        let r = |m: String| try_action(&c, send("ref-finance-101.testnet", &m), NOW + 1000);
        assert_eq!(r(swap(7, None, "1")), Err("not an allowed Ref pool"));
        assert_eq!(r(swap(1084, None, "0")), Err("a Ref swap needs a minimum output"));
        assert_eq!(r(swap(1084, Some(("swap_out_recipient", "thief.testnet")), "1")), Err("Ref swap fields other than pool, tokens and amounts"));
        assert_eq!(r(serde_json::json!({ "actions": [], "referral_id": "thief.testnet" }).to_string()), Err("Ref message fields other than actions"));
    }

    #[test]
    fn message_to_another_contract_refused() {
        ctx(owner(), NearToken::ZERO, NOW);
        let mut c = Extension::new(owner(), signer(), DELAY);
        let mut g = grant();
        g.near_venues.push(NearVenue { receiver_id: "other.testnet".parse().unwrap(), method: "ft_on_transfer".into() });
        ctx(wallet(), GRANT_DEPOSIT, NOW);
        c.grant_session("agent-1".into(), g);
        assert_eq!(try_action(&c, send("other.testnet", "{\"anything\":1}"), NOW + 1000), Err("only a plain deposit (empty msg) to this contract"));
    }
}


mod rhea_lending {
    use super::*;
    const LENDING: &str = "contract.1689937928.burrow.testnet";
    const ORACLE: &str = "mock-priceoracle.testnet";
    const USDC: &str = "usdc.fakes.testnet";

    fn setup() -> Extension {
        ctx(owner(), NearToken::ZERO, NOW);
        let mut c = Extension::new(owner(), signer(), DELAY);
        let mut g = grant();
        for m in ["execute_with_pyth", "ft_on_transfer"] {
            g.near_venues.push(NearVenue { receiver_id: LENDING.parse().unwrap(), method: m.into() });
        }
        g.near_venues.push(NearVenue { receiver_id: ORACLE.parse().unwrap(), method: "oracle_call".into() });
        g.near_venues.push(NearVenue { receiver_id: "ref-finance-101.testnet".parse().unwrap(), method: "pool:1".into() });
        g.near_venues.push(NearVenue { receiver_id: "ref-finance-101.testnet".parse().unwrap(), method: "pool:2".into() });
        g.caps.push(Cap { asset: format!("borrow:near:{USDC}"), daily: U128(1_000_000) });
        ctx(wallet(), GRANT_DEPOSIT, NOW);
        c.grant_session("agent-1".into(), g);
        c
    }
    fn call(to: &str, method: &str, args: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "type": "near_call", "receiver_id": to, "method": method, "args": args.to_string(), "deposit": "1" })
    }
    fn pyth(actions: serde_json::Value) -> serde_json::Value {
        call(LENDING, "execute_with_pyth", serde_json::json!({ "actions": actions }))
    }
    fn ft(to: &str, msg: serde_json::Value) -> serde_json::Value {
        call(&token().to_string(), "ft_transfer_call", serde_json::json!({ "receiver_id": to, "amount": "10", "msg": msg.to_string() }))
    }

    #[test]
    fn borrow_counts_against_its_cap() {
        let c = setup();
        let borrow = |n: &str| pyth(serde_json::json!([{ "Borrow": { "token_id": USDC, "amount": n } }, { "Withdraw": { "token_id": USDC, "max_amount": n } }]));
        assert_eq!(try_action(&c, borrow("1000000"), NOW + 1000), Ok(()));
        assert_eq!(try_action(&c, borrow("1000001"), NOW + 1000), Err("over the daily cap"));
        // Through the oracle, too.
        let via = call(ORACLE, "oracle_call", serde_json::json!({ "receiver_id": LENDING, "msg": serde_json::json!({ "actions": [{ "Borrow": { "token_id": USDC, "amount": "1000001" } }] }).to_string() }));
        assert_eq!(try_action(&c, via, NOW + 1000), Err("over the daily cap"));
    }

    #[test]
    fn nothing_goes_elsewhere() {
        let c = setup();
        let w = pyth(serde_json::json!([{ "Withdraw": { "token_id": USDC, "max_amount": "1", "receiver_id": "thief.testnet" } }]));
        assert_eq!(try_action(&c, w, NOW + 1000), Err("lending action fields other than token and amounts"));
        let via = call(ORACLE, "oracle_call", serde_json::json!({ "receiver_id": "thief.testnet", "msg": "{}" }));
        assert_eq!(try_action(&c, via, NOW + 1000), Err("the oracle may only call Rhea Lending"));
        // Tokens sent in may only become collateral or repay a loan.
        let borrow_in = ft(LENDING, serde_json::json!({ "Execute": { "actions": [{ "Borrow": { "token_id": USDC, "amount": "1" } }] } }));
        assert_eq!(try_action(&c, borrow_in, NOW + 1000), Err("a lending action that isn't allowed here"));
        let ok = ft(LENDING, serde_json::json!({ "Execute": { "actions": [{ "IncreaseCollateral": { "token_id": token().to_string(), "max_amount": "10" } }] } }));
        assert_eq!(try_action(&c, ok, NOW + 1000), Ok(()));
    }

    #[test]
    fn multi_hop_swap_minimum_on_the_last_hop() {
        let c = setup();
        let hop = |p: u64, min: &str| serde_json::json!({ "pool_id": p, "token_in": "a.testnet", "token_out": "b.testnet", "min_amount_out": min });
        let ok = ft("ref-finance-101.testnet", serde_json::json!({ "force": 0, "skip_unwrap_near": false, "actions": [hop(1, "0"), hop(2, "5")] }));
        assert_eq!(try_action(&c, ok, NOW + 1000), Ok(()));
        let bad = ft("ref-finance-101.testnet", serde_json::json!({ "force": 0, "actions": [hop(1, "5"), hop(2, "0")] }));
        assert_eq!(try_action(&c, bad, NOW + 1000), Err("a Ref swap needs a minimum output"));
    }
}

mod dogecoin {
    use super::*;
    use crate::utxo;

    /// A legacy transaction with one input and two outputs, the second to `script`.
    fn prev_tx(script: &str, value: u64) -> Vec<u8> {
        let mut t = 1u32.to_le_bytes().to_vec();
        t.push(1);
        t.extend([7u8; 32]);
        t.extend(0u32.to_le_bytes());
        t.push(0);
        t.extend(0xffff_ffffu32.to_le_bytes());
        t.push(2);
        let other = hex::decode("76a914000000000000000000000000000000000000000088ac").unwrap();
        t.extend(5u64.to_le_bytes());
        t.push(other.len() as u8);
        t.extend(other);
        let s = hex::decode(script).unwrap();
        t.extend(value.to_le_bytes());
        t.push(s.len() as u8);
        t.extend(s);
        t.extend(0u32.to_le_bytes());
        t
    }
    fn txid(t: &[u8]) -> [u8; 32] {
        let mut h: [u8; 32] = Sha256::digest(Sha256::digest(t)).into();
        h.reverse();
        h
    }

    #[test]
    fn reads_the_real_amount() {
        ctx(owner(), NearToken::ZERO, NOW);
        let t = prev_tx(P2PKH_SELF, 123_456);
        assert_eq!(utxo::prev_output(&t, &txid(&t), 1).unwrap(), (123_456, hex::decode(P2PKH_SELF).unwrap()));
        // A different transaction than the txid names.
        let mut wrong = t.clone();
        wrong[5] ^= 1;
        assert_eq!(utxo::prev_output(&wrong, &txid(&t), 1).err(), Some("previous transaction doesn't match the input's txid"));
        assert_eq!(utxo::prev_output(&t, &txid(&t), 9).err(), Some("no such output"));
    }

    #[test]
    fn understated_input_refused() {
        ctx(owner(), NearToken::ZERO, NOW);
        let mut c = Extension::new(owner(), signer(), DELAY);
        let mut g = grant();
        g.utxo = vec![UtxoGrant { chain: "dogecoin".into(), self_script: P2PKH_SELF.into(), recipients: vec![] }];
        g.caps.push(Cap { asset: "utxo:dogecoin".into(), daily: U128(100_000_000_000) });
        g.queue_new_targets = true;
        ctx(wallet(), GRANT_DEPOSIT, NOW);
        c.grant_session("agent-1".into(), g);
        let t = prev_tx(P2PKH_SELF, 50_000_000_000);
        let id = hex::encode(txid(&t));
        let action = |value: u64, prev: Option<String>| serde_json::json!({ "type": "utxo_pay", "chain": "dogecoin",
            "inputs": [{ "txid": id, "vout": 1, "value": value, "prev_tx": prev }],
            "outputs": [{ "script": P2PKH_SELF, "value": 1_000_000_000 }] });
        // The real amount: fine (all change, so only the fee leaves).
        assert_eq!(try_action(&c, action(50_000_000_000, Some(hex::encode(&t))), NOW + 1000), Ok(()));
        // Claiming the coin is smaller than it is, to hide a huge fee.
        assert_eq!(try_action(&c, action(1_100_000_000, Some(hex::encode(&t))), NOW + 1000), Err("input isn't the account's coin of that amount"));
        assert_eq!(try_action(&c, action(50_000_000_000, None), NOW + 1000), Err("Dogecoin inputs need their previous transaction"));
    }
}

mod tron_chain {
    use super::*;
    use crate::rules::{TronGrant, TronVenue};
    use crate::tron::{self, TronTx};

    const ME: &str = "THqBvSMndYYoZzmXmA1obnfGMSPRJ9oqdR";
    const JTRX: &str = "TKM7w4qFmkXQLEF2MgrQroBYpd5TY7i1pq";
    const ROUTER: &str = "TYMjxCXfqLpMWW1QToP6hbcjpion7EE25p";
    const WTRX: &str = "TYsbWxNnyTgsZaTFaue9hqpxkU3Fkco94a";
    const FRIEND: &str = "TJUCStq3WqfKqZLuZje5v7z6Ua6iBry1P6";

    fn tg() -> TronGrant {
        TronGrant {
            self_address: ME.into(),
            venues: vec![
                TronVenue { contract: JTRX.into(), selector: "0x1249c58b".into() },
                TronVenue { contract: JTRX.into(), selector: "0xc5ebeaec".into() },
                TronVenue { contract: ROUTER.into(), selector: "0x7ff36ab5".into() },
            ],
            recipients: vec![FRIEND.into()],
            nfts: vec![],
        }
    }
    fn tx(call: serde_json::Value) -> TronTx {
        serde_json::from_value(serde_json::json!({ "ref_block_bytes": "baf4", "ref_block_hash": "4d5c23452999646b",
            "expiration": "1794525327000", "timestamp": "1794525269886", "fee_limit": "100000000", "call": call })).unwrap()
    }

    /// The same call built by a Nile node (wallet/triggersmartcontract): identical raw_data and ID.
    #[test]
    fn matches_node() {
        ctx(owner(), NearToken::ZERO, NOW);
        let (raw, id, spends) = tron::check(&tg(), &tx(serde_json::json!({ "kind": "trigger", "contract": JTRX, "call_value": "1000000", "data": "0x1249c58b" })), false).unwrap();
        assert_eq!(hex::encode(raw), "0a02baf422084d5c23452999646b4098ddad909d345a71081f126d0a31747970652e676f6f676c65617069732e636f6d2f70726f746f636f6c2e54726967676572536d617274436f6e747261637412380a1541563cc6fdde0981931f2eaea8fbb2e829d3244b6912154166de0d4dbe8ca9865423ceccacc699c682d4cc9418c0843d22041249c58b70fe9eaa909d34900180c2d72f");
        assert_eq!(hex::encode(id), "aaa3215f07397ea29ec5f8e92a808da8c424dc32e64d40f8aedb6ad547b035e2");
        // The supplied TRX plus the whole fee limit.
        assert_eq!(spends, vec![("trx".to_string(), 101_000_000)]);
    }

    fn word(a: &str) -> String { format!("{:0>64}", hex::encode(&tron::address(a).unwrap()[1..])) }

    #[test]
    fn swap_output_must_come_back() {
        ctx(owner(), NearToken::ZERO, NOW);
        // swapExactETHForTokens(amountOutMin, path, to, deadline), path [WTRX, x].
        let swap = |to: &str| format!("0x7ff36ab5{:064x}{:064x}{}{:064x}{:064x}{}{}", 1, 128, word(to), 9_999_999_999u64, 2, word(WTRX), word(JTRX));
        let ok = tron::check(&tg(), &tx(serde_json::json!({ "kind": "trigger", "contract": ROUTER, "call_value": "5", "data": swap(ME) })), false);
        assert!(ok.is_ok());
        let bad = tron::check(&tg(), &tx(serde_json::json!({ "kind": "trigger", "contract": ROUTER, "call_value": "5", "data": swap(FRIEND) })), false);
        assert_eq!(bad.map(|_| ()), Err("venue call sends the result to someone else"));
    }

    #[test]
    fn borrow_counted_and_refusals() {
        ctx(owner(), NearToken::ZERO, NOW);
        let (_, _, s) = tron::check(&tg(), &tx(serde_json::json!({ "kind": "trigger", "contract": JTRX, "data": format!("0xc5ebeaec{:064x}", 7) })), false).unwrap();
        assert!(s.contains(&(format!("borrow:tron:{JTRX}"), 7)));
        let pay = |to: &str| tron::check(&tg(), &tx(serde_json::json!({ "kind": "transfer", "to": to, "amount": "1" })), false).map(|_| ());
        assert_eq!(pay(FRIEND), Ok(()));
        assert_eq!(pay(ROUTER), Err("recipient not allowed"));
        // redeem isn't a listed venue.
        let r = tron::check(&tg(), &tx(serde_json::json!({ "kind": "trigger", "contract": JTRX, "data": format!("0xdb006a75{:064x}", 1) })), false);
        assert_eq!(r.map(|_| ()), Err("not an allowed venue"));
    }
}

mod ton_chain {
    use crate::rules::{TonGrant, TonJetton};
    use crate::ton::{self, TonMsg, TonTx};
    use crate::ton_cell::{self, Cell};
    use near_sdk::json_types::{U128, U64};

    // The demo account's wallet (v4r2 of its MPC key), EVAA's testnet
    // classic master (@evaafi/sdk 1.0.3), its jUSDC test jetton.
    const ME: &str = "0QATZwvqCTfh9MWmVH-mFtTAHPpXaSNf7GX2eCWczYkjUZTx";
    const EVAA: &str = "kQBTmXNOP6TUEQ-v85mlefDnVUor6tX_ZYFvQnaoxkM7LFVl";
    const JUSDC: &str = "kQDaY5yUatYnHei73HBqRX_Ox9LK2XnR7XuCY9MFC2INbfYI";
    const FRIEND: &str = "kQBe4gtSQMxM5RpMYLr4ydNY72F8JkY-icZXG1NJcsju8XM7";
    const JUSDC_ID: &str = "83d916c68510802104d1f75aa6ce30eb1e477aede0d380eee2188e0e56581fc6";

    fn grant() -> TonGrant {
        TonGrant {
            self_address: ME.into(),
            recipients: vec![FRIEND.into()],
            // Any address stands in for the jetton wallet in these tests.
            jettons: vec![TonJetton { master: JUSDC.into(), wallet: FRIEND.into() }, TonJetton { master: TSTON.into(), wallet: TSTON_WALLET.into() }],
            evaa: vec![EVAA.into()],
            lst: vec![crate::rules::TonLst { pool: POOL.into(), jetton: TSTON.into() }],
            nft: false,
        }
    }

    /// Tonstakers' testnet pool (tonstakers-sdk 0.0.20) and its tsTON
    /// master (the jetton master the pool administers, on toncenter).
    const POOL: &str = "kQANFsYyYn-GSZ4oajUJmboDURZU-udMHf9JxzO4vYM_hFP3";
    const TSTON: &str = "kQAiQ2XK7BXePLwemeo-u4wNyjg-wxGeySmaFGEP7R2Mhf4m";
    /// Stands in for the account's tsTON wallet.
    const TSTON_WALLET: &str = "kQBe4gtSQMxM5RpMYLr4ydNY72F8JkY-icZXG1NJcsju8XM7";

    #[test]
    fn tonstakers_bodies_match_the_sdk() {
        // @ton/core, as tonstakers-sdk 0.0.20 builds them: stake with query id
        // 7 and partner code 0; unstake 2 tsTON, response to the account,
        // fill_or_kill.
        let me = ton::address(ME).unwrap();
        assert_eq!(hex::encode(ton::lst_stake_body(7).unwrap().hash()), "b6e8944d92d0119338ed2cda0eb41a723f33fa028c4bef832871dea90b4dfda4");
        assert_eq!(hex::encode(ton::lst_unstake_body(7, 2_000_000_000, &me, false, true).unwrap().hash()), "1e93b43e52da04b991533bf11333c96f06095735d51f43047871555d66caf0e9");
    }

    #[test]
    fn nft_transfer() {
        // TEP-62 transfer as @ton/core builds it: query id 7, to FRIEND,
        // response to the account, forward 1 nanoton.
        let me = ton::address(ME).unwrap();
        assert_eq!(hex::encode(ton::nft_transfer_body(7, &ton::address(FRIEND).unwrap(), &me, 1).unwrap().hash()), "5d6f0622fc6887e754525dbdd21c7bc6ca5f24e3c128be528d5c649d7ed79d71");
        let mut g = grant();
        let t = |to: &str| tx(vec![TonMsg::NftTransfer { item: EVAA.into(), to: to.into(), ton: U64(50_000_000), forward_ton: U64(1) }]);
        assert_eq!(ton::check(&g, &t(FRIEND), false).err(), Some("this grant doesn't allow moving NFTs"));
        g.nft = true;
        let (_, _, spends) = ton::check(&g, &t(FRIEND), false).unwrap();
        assert_eq!(spends, vec![("nft:ton".to_string(), 1), ("ton".to_string(), 50_000_000)]);
        assert_eq!(ton::check(&g, &t(POOL), false).err(), Some("recipient not allowed"));
    }

    #[test]
    fn tonstakers_rules() {
        let g = grant();
        let (_, _, spends) = ton::check(&g, &tx(vec![TonMsg::LstStake { pool: POOL.into(), amount: U128(2_000_000_000), ton: U64(1_000_000_000) }]), false).unwrap();
        assert_eq!(spends, vec![("ton".to_string(), 3_000_000_000)]);
        // Another pool, even queued: refused.
        assert_eq!(ton::check(&g, &tx(vec![TonMsg::LstStake { pool: EVAA.into(), amount: U128(1), ton: U64(1) }]), true).err(), Some("not an allowed staking pool"));
        // Unstake burns from the account's tsTON wallet, counted on the jetton.
        let u = TonMsg::LstUnstake { pool: POOL.into(), amount: U128(5), ton: U64(1_050_000_000), wait_till_round_end: false, fill_or_kill: false };
        let (m, _, spends) = ton::check(&g, &tx(vec![u]), false).unwrap();
        assert_eq!(spends[0], (format!("jetton:{}", ton_cell::friendly(&ton::address(TSTON).unwrap(), true, true)), 5));
        assert_eq!(m.refs.len(), 1);
        // No pairing for the pool's jetton on the grant: refused.
        let mut g2 = grant();
        g2.jettons.retain(|j| j.master != TSTON);
        let u = TonMsg::LstUnstake { pool: POOL.into(), amount: U128(5), ton: U64(1), wait_till_round_end: false, fill_or_kill: false };
        assert_eq!(ton::check(&g2, &tx(vec![u]), false).err(), Some("jetton not allowed"));
    }

    #[test]
    fn evaa_bodies_match_the_sdk() {
        // Hashes from @evaafi/sdk 1.0.3 (supplies with includeUserCode and
        // customPayloadRecipient = the account, as the masters require):
        // supply 2 TON; supply 3 jUSDC (forward 0.25 TON); withdraw 1 jUSDC
        // with a stand-in price cell; query id 7.
        let me = ton::address(ME).unwrap();
        let pool = ton::address(EVAA).unwrap();
        assert_eq!(hex::encode(ton::evaa_supply(Some((7, 2_000_000_000)), &me).unwrap().hash()), "05f03108c4a7e1df94abc6d9f1a84049e874232105245b0d4f2aadf8b49a0263");
        let j = ton::jetton_transfer(7, 3_000_000, &pool, &me, 250_000_000, Some(ton::evaa_supply(None, &me).unwrap())).unwrap();
        assert_eq!(hex::encode(j.hash()), "88ed34bb0af4911961b85c240c6398e504e38f26c1a7991c856c8a29f34c42f3");
        let prices = Cell::from_boc(&hex::decode("b5ee9c724101010100030000022a35c1830a").unwrap()).unwrap();
        let id: [u8; 32] = hex::decode(JUSDC_ID).unwrap().try_into().unwrap();
        assert_eq!(hex::encode(ton::evaa_withdraw(7, &id, 1_000_000, &me, Some(prices)).unwrap().hash()), "1cc34fde96a52e1a787b3d8b6352716b8f3730a19974a02d6d25dadfde56ec38");
        // Without prices (EvaaMasterPyth, testnet): withdraw 0.3 TON.
        let ton_id: [u8; 32] = hex::decode("1a4219fe5e60d63af2a3cc7dce6fec69b45c6b5718497a6148e7c232ac87bd8a").unwrap().try_into().unwrap();
        assert_eq!(hex::encode(ton::evaa_withdraw(7, &ton_id, 300_000_000, &me, None).unwrap().hash()), "de671ed1f2091622b7ce040d20a330df0be6fda4344c6c2d646e0f3ede36def8");
    }

    fn tx(messages: Vec<TonMsg>) -> TonTx {
        TonTx { seqno: 3, valid_until: 1_800_000_000, messages }
    }

    #[test]
    fn ton_rules() {
        let g = grant();
        // A listed payment: counted against `ton`; the signing message is
        // what core's wallet code builds (same layout, mode 3).
        let (m, h, spends) = ton::check(&g, &tx(vec![TonMsg::Pay { to: FRIEND.into(), amount: U128(5), comment: Some("hi".into()) }]), false).unwrap();
        assert_eq!(h, m.hash());
        assert_eq!(spends, vec![("ton".to_string(), 5)]);
        // Supplying TON counts the amount and the gas.
        let (_, _, spends) = ton::check(&g, &tx(vec![TonMsg::EvaaSupply { master: EVAA.into(), asset: "ton".into(), amount: U128(2_000_000_000), ton: U64(250_000_000), forward_ton: U64(0) }]), false).unwrap();
        assert_eq!(spends, vec![("ton".to_string(), 2_250_000_000)]);
        // Withdrawing counts against the borrow cap.
        let w = TonMsg::EvaaWithdraw { master: EVAA.into(), asset_id: JUSDC_ID.into(), amount: U64(1_000_000), prices: "b5ee9c724101010100030000022a35c1830a".into(), ton: U64(480_000_000) };
        let (_, _, spends) = ton::check(&g, &tx(vec![w]), false).unwrap();
        assert_eq!(spends[0], (format!("borrow:ton:{JUSDC_ID}"), 1_000_000));
        // Jettons go only from the listed wallet, only to listed recipients.
        let (m, _, spends) = ton::check(&g, &tx(vec![TonMsg::Jetton { jetton: JUSDC.into(), to: FRIEND.into(), amount: U128(9), ton: U64(50_000_000), forward_ton: U64(1) }]), false).unwrap();
        assert_eq!(spends[0], (format!("jetton:{}", ton_cell::friendly(&ton::address(JUSDC).unwrap(), true, true)), 9));
        assert_eq!(m.refs.len(), 1);

        // Refused:
        let refuse = |msgs: Vec<TonMsg>| ton::check(&g, &tx(msgs), false).unwrap_err();
        assert_eq!(refuse(vec![TonMsg::Pay { to: EVAA.into(), amount: U128(5), comment: None }]), "recipient not allowed");
        assert_eq!(refuse(vec![TonMsg::Pay { to: ME.into(), amount: U128(5), comment: None }]), "payment to itself");
        assert_eq!(refuse(vec![TonMsg::Jetton { jetton: FRIEND.into(), to: FRIEND.into(), amount: U128(9), ton: U64(50_000_000), forward_ton: U64(0) }]), "jetton not allowed");
        assert_eq!(refuse(vec![TonMsg::Jetton { jetton: JUSDC.into(), to: EVAA.into(), amount: U128(9), ton: U64(50_000_000), forward_ton: U64(0) }]), "recipient not allowed");
        assert_eq!(refuse(vec![TonMsg::EvaaSupply { master: FRIEND.into(), asset: "ton".into(), amount: U128(1), ton: U64(1), forward_ton: U64(0) }]), "not an allowed lending pool");
        assert_eq!(refuse(vec![TonMsg::Pay { to: FRIEND.into(), amount: U128(0), comment: None }]), "amount must be more than zero");
        assert_eq!(refuse(vec![TonMsg::EvaaSupply { master: EVAA.into(), asset: "ton".into(), amount: U128(1), ton: U64(2_000_000_000), forward_ton: U64(0) }]), "gas must be 1 nanoton to 1 TON");
        assert_eq!(refuse(vec![]), "1 to 4 messages");
        let five = (0..5).map(|_| TonMsg::Pay { to: FRIEND.into(), amount: U128(1), comment: None }).collect();
        assert_eq!(refuse(five), "1 to 4 messages");
    }
}

mod blend {
    use crate::rules::StellarGrant;
    use crate::stellar::{self, StellarTx};

    // The demo account; Blend's testnet pool and tokens
    // (blend-capital/blend-utils testnet.contracts.json: TestnetV2, XLM, USDC).
    const ME: &str = "GDKKV36ZUBM4EOZL7DOYLKG7CNVKCYYI6XTYAHQY5LDRH6EOUQ4EMAPE";
    const POOL: &str = "CCEBVDYM32YNYCVNRXQKDFFPISJJCV557CDZEIRBEE4NCV4KHPQ44HGF";
    const XLM: &str = "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC";
    const USDC: &str = "CAQCFVLOBK5GIULPNZRGATJJMIZL5BSP7X5YJVMGCPTUEPFM4AVSRCJU";
    const DATA: &str = "000000000000000000000000000f4240000007d000000bb8000000000007a120";

    fn g() -> StellarGrant {
        StellarGrant { self_address: ME.into(), assets: vec![], recipients: vec![], dex: false, amm: false, blend: vec![POOL.into()] }
    }
    fn tx(requests: serde_json::Value, data: Option<&str>, fee: u32) -> StellarTx {
        serde_json::from_value(serde_json::json!({ "seq_num": "100", "fee": fee, "max_time": "1800000000",
            "ops": [{ "kind": "blend_submit", "pool": POOL, "requests": requests }], "soroban_data": data })).unwrap()
    }

    #[test]
    fn submit_matches_the_sdk() {
        // @stellar/stellar-sdk 13: submit(me, me, me, [collateral 10 XLM,
        // borrow 0.5 USDC]) with source-account auth over it and the pool's
        // XLM transfer under it, fee 600000, resource fee 500000.
        assert_eq!(stellar::contract_strkey(&stellar::xlm_contract(stellar::TESTNET)), XLM);
        let t = tx(serde_json::json!([{ "kind": "supply_collateral", "asset": XLM, "amount": "100000000" }, { "kind": "borrow", "asset": USDC, "amount": "5000000" }]), Some(DATA), 600_000);
        let (_, hash, spends) = stellar::check(&g(), &t, false, stellar::TESTNET).unwrap();
        assert_eq!(hex::encode(hash), "fa8a102a2077b99999a61665259ea0230cc8afafc1586573b072f49ff0a72a84");
        assert_eq!(spends, vec![("xlm".to_string(), 100_600_000), (format!("borrow:stellar:{USDC}"), 5_000_000)]);
    }

    #[test]
    fn blend_rules() {
        let one = |kind: &str| serde_json::json!([{ "kind": kind, "asset": XLM, "amount": "1" }]);
        // Withdrawals pay the account: only the fee is spent.
        let (_, _, spends) = stellar::check(&g(), &tx(one("withdraw_collateral"), Some(DATA), 600_000), false, stellar::TESTNET).unwrap();
        assert_eq!(spends, vec![("xlm".to_string(), 600_000)]);
        let err = |t: StellarTx| stellar::check(&g(), &t, false, stellar::TESTNET).unwrap_err();
        let mut other = tx(one("supply"), Some(DATA), 600_000);
        if let stellar::StellarOp::BlendSubmit { pool, .. } = &mut other.ops[0] {
            *pool = USDC.into();
        }
        assert_eq!(err(other), "not an allowed lending pool");
        assert_eq!(err(tx(one("liquidate"), Some(DATA), 600_000)), "unknown Blend request");
        assert_eq!(err(tx(one("supply"), None, 600_000)), "a Soroban call needs its resources (soroban_data)");
        assert_eq!(err(tx(one("supply"), Some(DATA), 400_000)), "the fee doesn't cover the resource fee");
        assert_eq!(err(tx(one("supply"), Some(DATA), 30_000_000)), "fee outside 1 stroop to 2 XLM");
    }
}

// ---- NEAR Intents top-ups (intents.rs) ----
// Quotes 1Click signed on mainnet, 13 November 2026: 20 USDC on Base to ETH on
// Arbitrum, paying the account itself (`self`), a stranger (`friend`), a dry
// quote (`dry`), and 0.1 SOL to USDC on Base (`sol`).
mod intents_topup {
    use super::*;
    use crate::intents;

    const BASE: u64 = 8453;
    const BASE_USDC: &str = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
    const DEPOSIT: &str = "0x48b2907a4cdb90ae9620a25ebe71a8b6ce003c45";
    /// The `self` quote's timestamp, plus a minute.
    const T: u64 = 1_794_542_931_524 + 60_000;

    fn fixture(name: &str) -> (String, String) {
        let raw = match name {
            "self" => include_str!("testdata/intents-self.json"),
            "friend" => include_str!("testdata/intents-friend.json"),
            "dry" => include_str!("testdata/intents-dry.json"),
            _ => include_str!("testdata/intents-sol.json"),
        };
        let v: serde_json::Value = serde_json::from_str(raw).unwrap();
        (v["quote"].as_str().unwrap().into(), v["signature"].as_str().unwrap().into())
    }
    fn setup() -> Extension {
        ctx(owner(), NearToken::ZERO, T);
        let mut c = Extension::new(owner(), signer(), DELAY);
        let mut g = grant();
        g.expires_at_ms = T + 7 * DAY;
        g.caps.push(Cap { asset: format!("evm:{BASE}:{BASE_USDC}"), daily: U128(100_000_000) });
        ctx(wallet(), GRANT_DEPOSIT, T);
        c.grant_session("agent-1".into(), g);
        c
    }
    fn usdc_transfer(to: &str, amount: u128) -> serde_json::Value {
        serde_json::json!({ "type": "evm_tx", "tx": {
            "chain_id": BASE, "nonce": 0, "max_priority_fee_per_gas": "1000000",
            "max_fee_per_gas": "2000000000", "gas_limit": 100000, "to": BASE_USDC,
            "value": "0", "data": format!("0xa9059cbb{}{}", word_addr(to), word_u(amount))
        }})
    }
    fn topup(name: &str, inner: serde_json::Value) -> serde_json::Value {
        let (quote, signature) = fixture(name);
        serde_json::json!({ "type": "intents_deposit", "quote": quote, "signature": signature, "action": inner })
    }

    #[test]
    fn signature_and_times_as_1click_computes_them() {
        let (q, _) = fixture("self");
        // base58(sha256(quote)), as one-click-sdk's quoteHash gives.
        assert!(intents::signed_message(&q).len() >= 43);
        assert_eq!(intents::iso_ms("2026-11-13T04:08:51.524Z"), Ok(1_794_542_931_524));
        assert_eq!(intents::iso_ms("2026-11-16T04:18:51Z"), Ok(1_794_802_731_000));
        assert_eq!(intents::usd_micros("19.990380000000"), Ok(19_990_380));
    }

    #[test]
    fn top_up_to_the_account_itself() {
        let c = setup();
        assert_eq!(try_action(&c, topup("self", usdc_transfer(DEPOSIT, 20_000_000)), T), Ok(()));
    }

    #[test]
    fn deposit_address_only_with_the_quote() {
        let c = setup();
        assert_eq!(try_action(&c, usdc_transfer(DEPOSIT, 20_000_000), T), Err("recipient not allowed"));
    }

    #[test]
    fn more_than_quoted_refused() {
        let c = setup();
        assert_eq!(try_action(&c, topup("self", usdc_transfer(DEPOSIT, 20_000_001)), T), Err("more than the quoted amount"));
    }

    #[test]
    fn another_asset_refused() {
        let c = setup();
        // USDC on Base Sepolia to an allowed recipient: not the quoted asset.
        let mut t = usdc_transfer(FRIEND, 1_000);
        t["tx"]["chain_id"] = serde_json::json!(BASE_SEPOLIA);
        t["tx"]["to"] = serde_json::json!(USDC);
        assert_eq!(try_action(&c, topup("self", t), T), Err("the transfer doesn't move the quoted asset"));
    }

    #[test]
    fn tampered_quote_refused() {
        let c = setup();
        let (q, s) = fixture("self");
        let q = q.replace("0x48B2907A4cDb90ae9620a25EBe71a8b6ce003c45", "0x6666666666666666666666666666666666666666");
        let a = serde_json::json!({ "type": "intents_deposit", "quote": q, "signature": s, "action": usdc_transfer(STRANGER, 20_000_000) });
        assert_eq!(try_action(&c, a, T), Err("quote not signed by 1Click"));
    }

    #[test]
    fn paying_someone_else_refused() {
        let c = setup();
        assert_eq!(try_action(&c, topup("friend", usdc_transfer(DEPOSIT, 20_000_000)), T), Err("a top-up pays only the account itself"));
    }

    #[test]
    fn dry_quote_refused() {
        let c = setup();
        assert_eq!(try_action(&c, topup("dry", usdc_transfer(DEPOSIT, 20_000_000)), T), Err("a dry quote has no deposit address"));
    }

    #[test]
    fn old_quote_refused() {
        let c = setup();
        assert_eq!(try_action(&c, topup("self", usdc_transfer(DEPOSIT, 20_000_000)), T + 31 * 60_000), Err("quote too old"));
    }

    #[test]
    fn no_nesting() {
        let c = setup();
        let inner = topup("self", usdc_transfer(DEPOSIT, 20_000_000));
        assert_eq!(try_action(&c, topup("self", inner), T), Err("one top-up at a time"));
    }

    #[test]
    fn solana_origin() {
        let (q, s) = fixture("sol");
        let mut g = grant();
        g.sol = Some(rules::SolGrant { self_address: "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM".into(), venues: vec![], recipients: vec![], nft_floor: U128(0), perps: vec![] });
        ctx(owner(), NearToken::ZERO, T);
        let d = intents::check(&g, &q, &s, 1_794_542_933_875 + 1000, &wallet()).unwrap();
        assert_eq!(d.spend, "sol");
        assert_eq!(d.amount_in, 100_000_000);
        assert_eq!(d.address, "9PUh5nN4t9TmZJWfUgZfnq4k7q15cSpnru3S2rQqsGTG");
        // Refunds to an address that isn't the account's: refused.
        g.sol.as_mut().unwrap().self_address = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB".into();
        assert_eq!(intents::check(&g, &q, &s, 1_794_542_933_875 + 1000, &wallet()), Err("refunds must go to the account itself"));
    }
}

// ---- Cardano agent payments (cardano.rs) ----
mod cardano_pay {
    use super::*;

    /// addr_test1vz2fxv… (enterprise) as the account, addr_test1qz2fxv… (base)
    /// as a listed recipient, raw address bytes.
    const ME: &str = "609493315cd92eb5d8c4304e67b7e16ae36d61d34502694657811a2c8e";
    const FRIEND: &str = "009493315cd92eb5d8c4304e67b7e16ae36d61d34502694657811a2c8e32c728d3861e164cab28cb8f006448139c8f1740ffb8e7aa9e5232dc";
    const STRANGER: &str = "00aa93315cd92eb5d8c4304e67b7e16ae36d61d34502694657811a2c8e32c728d3861e164cab28cb8f006448139c8f1740ffb8e7aa9e5232dc";

    fn setup() -> Extension {
        ctx(owner(), NearToken::ZERO, NOW);
        let mut c = Extension::new(owner(), signer(), DELAY);
        let mut g = grant();
        g.utxo = vec![UtxoGrant { chain: "cardano".into(), self_script: ME.into(), recipients: vec![FRIEND.into()] }];
        g.caps.push(Cap { asset: "utxo:cardano".into(), daily: U128(5_000_000) });
        ctx(wallet(), GRANT_DEPOSIT, NOW);
        c.grant_session("agent-1".into(), g);
        c
    }
    fn pay(to: &str, amount: u64, change: u64) -> serde_json::Value {
        serde_json::json!({ "type": "utxo_pay", "chain": "cardano", "expiry_height": 90_000_000u32,
            "inputs": [{ "txid": "aa".repeat(32), "vout": 1, "value": 10_000_000u64 }],
            "outputs": [{ "script": to, "value": amount }, { "script": ME, "value": change }] })
    }

    #[test]
    fn body_hash_matches_core() {
        // near/core cardano_plan for the same send: 2 ADA to FRIEND, change
        // 7.832211 ADA, fee 0.167789 ADA, TTL slot 90,000,000.
        let s = get(&setup());
        let a: Action = serde_json::from_value(pay(FRIEND, 2_000_000, 7_832_211)).unwrap();
        let p = rules::check(&s, &a, Mode::Direct, NOW + 1000, &wallet(), &ext(), &signer()).unwrap();
        let rules::Kind::Eddsa { path, message } = p.kind else { panic!("not an ed25519 signature") };
        assert_eq!(path, "cardano");
        assert_eq!(hex::encode(message), "6892ed18360cd88135110c876cf5ce57591b5156df1e3bbc9c26477375ee50fe");
        // Payment plus fee leave the account.
        assert_eq!(p.spends, vec![("utxo:cardano".to_string(), 2_167_789)]);
    }

    #[test]
    fn cardano_rules() {
        let c = setup();
        assert_eq!(try_action(&c, pay(STRANGER, 2_000_000, 7_832_211), NOW + 1000), Err("recipient not allowed"));
        // Over the daily cap of 5 ADA.
        assert_eq!(try_action(&c, pay(FRIEND, 6_000_000, 3_832_211), NOW + 1000), Err("over the daily cap"));
        // A fee over 2 ADA (most of the input left unaccounted).
        assert_eq!(try_action(&c, pay(FRIEND, 2_000_000, 1_000_000), NOW + 1000), Err("fee must be above 0 and at most 2 ADA"));
        let mut a = pay(FRIEND, 2_000_000, 7_832_211);
        a["expiry_height"] = serde_json::json!(0);
        assert_eq!(try_action(&c, a, NOW + 1000), Err("Cardano needs a TTL slot"));
    }

    fn get(c: &Extension) -> Session {
        c.get_session(wallet(), "agent-1".into()).unwrap()
    }
}

// ---- Jupiter (sol.rs): a real swap from Jupiter's API for the demo
// account (0.1 SOL to USDC, direct route, legacy transaction), 13 November
// 2026; the same swap ran on a Surfpool fork of mainnet. ----
mod jupiter_swap {
    use super::*;
    use crate::rules::{SolGrant, SolVenue};

    const JUP: &str = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";

    fn fixture() -> (String, serde_json::Value) {
        let v: serde_json::Value = serde_json::from_str(include_str!("testdata/jupiter-swap.json")).unwrap();
        (v["self_address"].as_str().unwrap().to_string(), v["action"].clone())
    }
    fn setup(venue: bool) -> Extension {
        let (me, _) = fixture();
        ctx(owner(), NearToken::ZERO, NOW);
        let mut c = Extension::new(owner(), signer(), DELAY);
        let mut g = grant();
        g.sol = Some(SolGrant { self_address: me, venues: if venue { vec![SolVenue { program: JUP.into(), scope: JUP.into() }] } else { vec![] }, recipients: vec![], nft_floor: U128(0), perps: vec![] });
        g.caps.push(Cap { asset: "sol".into(), daily: U128(1_000_000_000) });
        ctx(wallet(), GRANT_DEPOSIT, NOW);
        c.grant_session("agent-1".into(), g);
        c
    }
    fn swap_ix(a: &mut serde_json::Value) -> &mut serde_json::Value {
        a["instructions"].as_array_mut().unwrap().iter_mut().find(|i| i["program"] == JUP).unwrap()
    }
    /// Sets byte `from_end` (counted from the end) of the swap's data.
    fn set_tail(a: &mut serde_json::Value, from_end: usize, b: u8) {
        let ix = swap_ix(a);
        let mut d = hex::decode(ix["data"].as_str().unwrap()).unwrap();
        let n = d.len();
        d[n - from_end] = b;
        ix["data"] = serde_json::json!(hex::encode(d));
    }

    #[test]
    fn real_swap_passes() {
        let (_, a) = fixture();
        let s = setup(true).get_session(wallet(), "agent-1".into()).unwrap();
        let act: Action = serde_json::from_value(a).unwrap();
        let p = rules::check(&s, &act, Mode::Direct, NOW + 1000, &wallet(), &ext(), &signer()).unwrap();
        // 0.1 SOL in, plus the two new token accounts' rent and the fee.
        let sol = p.spends.iter().find(|(x, _)| x == "sol").unwrap().1;
        assert!(sol > 100_000_000 && sol < 110_000_000, "{sol}");
    }

    #[test]
    fn refusals() {
        let (_, a) = fixture();
        let c = setup(true);
        // Output to someone else's token account.
        let mut x = a.clone();
        swap_ix(&mut x)["accounts"][3]["pubkey"] = serde_json::json!("HH7S29UZLn5CSkrT6JtPRShtQtfqrBCqGz5rzqHFkLtE");
        assert!(try_action(&c, x, NOW + 1000).is_err());
        // A platform fee (the last byte).
        let mut x = a.clone();
        set_tail(&mut x, 1, 50);
        assert_eq!(try_action(&c, x, NOW + 1000), Err("no platform fee"));
        // Slippage over 3% (the two bytes before it: 500 bps).
        let mut x = a.clone();
        set_tail(&mut x, 3, 0xf4);
        set_tail(&mut x, 2, 0x01);
        assert_eq!(try_action(&c, x, NOW + 1000), Err("slippage over 3%"));
        // No Jupiter venue on the grant.
        assert_eq!(try_action(&setup(false), a, NOW + 1000), Err("not an allowed venue"));
    }
}

// ---- Polymarket (polymarket.rs) ----
mod polymarket_order {
    use super::*;
    use crate::polymarket;

    const CTF_EX: &str = "0x4bfb41d5b3570defd03c39a9a4d8de6bd8b8982e";
    const NEG_EX: &str = "0xc5d563a36ae78145c45a50134d48a1215220f80a";
    const TOKEN: &str = "71321045679252212594626385532706912750332728571942532289631379312455583992563";
    const Z: &str = "0x0000000000000000000000000000000000000000";

    fn order(ex: &str, side: u8) -> serde_json::Value {
        serde_json::json!({ "exchange": ex, "salt": "12345", "maker": SELF, "signer": SELF, "taker": Z, "token_id": TOKEN,
            "maker_amount": "5000000", "taker_amount": "10000000", "expiration": 1_800_086_400u64, "nonce": "0",
            "fee_rate_bps": "0", "side": side, "signature_type": 0 })
    }
    fn setup() -> Extension {
        ctx(owner(), NearToken::ZERO, NOW);
        let mut c = Extension::new(owner(), signer(), DELAY);
        let mut g = grant();
        g.evm_venues.push(EvmVenue { chain_id: 137, to: CTF_EX.into(), selector: "0x00000000".into() });
        g.caps.push(Cap { asset: format!("evm:137:{}", polymarket::USDC_E), daily: U128(20_000_000) });
        g.caps.push(Cap { asset: polymarket::SELL_CAP.into(), daily: U128(1_000_000) });
        ctx(wallet(), GRANT_DEPOSIT, NOW);
        c.grant_session("agent-1".into(), g);
        c
    }
    fn act(o: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "type": "polymarket_order", "order": o })
    }

    #[test]
    fn hash_matches_the_exchanges() {
        // hashOrder() of both exchanges on a fork of Polygon (13 November 2026).
        let o: polymarket::PmOrder = serde_json::from_value(order(CTF_EX, 0)).unwrap();
        assert_eq!(hex::encode(polymarket::order_hash(&o).unwrap()), "046c5c72f9bae5b22ea710e6e2866e0c90b686d5e8b7d47e069bf45617806656");
        let o: polymarket::PmOrder = serde_json::from_value(order(NEG_EX, 0)).unwrap();
        assert_eq!(hex::encode(polymarket::order_hash(&o).unwrap()), "6dc74cae771fa68b35aa3baae7f3ff689efad5d9d88fb2fbdc1f2394063e29f7");
    }

    #[test]
    fn rules() {
        let c = setup();
        let at = NOW + 1000;
        assert_eq!(try_action(&c, act(order(CTF_EX, 0)), at), Ok(()));
        // Not a venue on this grant.
        assert_eq!(try_action(&c, act(order(NEG_EX, 0)), at), Err("not an allowed venue"));
        // Selling 5 shares: over the 1-share sell limit.
        assert_eq!(try_action(&c, act(order(CTF_EX, 1)), at), Err("over the daily cap"));
        let mut o = order(CTF_EX, 0);
        o["maker"] = serde_json::json!(FRIEND);
        assert_eq!(try_action(&c, act(o), at), Err("the account must be maker and signer"));
        let mut o = order(CTF_EX, 0);
        o["taker"] = serde_json::json!(FRIEND);
        assert_eq!(try_action(&c, act(o), at), Err("only public orders"));
        let mut o = order(CTF_EX, 0);
        o["signature_type"] = serde_json::json!(1);
        assert_eq!(try_action(&c, act(o), at), Err("only the account's own signature (EOA)"));
        let mut o = order(CTF_EX, 0);
        o["expiration"] = serde_json::json!(1_900_000_000u64);
        assert_eq!(try_action(&c, act(o), at), Err("an order expires within 30 days"));
        let mut o = order(CTF_EX, 0);
        o["maker_amount"] = serde_json::json!("30000000");
        assert_eq!(try_action(&c, act(o), at), Err("over the daily cap"));
    }
}

// ---- Phoenix (sol.rs): SOL/USDC swaps; the same layout ran on the Surfpool fork ----
mod phoenix_swap {
    use super::*;
    use crate::rules::{SolGrant, SolVenue};

    const PHX: &str = "PhoeNiXZ8ByJGLkxNfZRnkUfjvmuYqLR89jjFHGqdXY";
    const MARKET: &str = "4DoNfFBfF7UokCC2FQzriy7yHK6DY6NVdYpuekQ5pRgg";

    /// The account and its wSOL and USDC token accounts, from the Jupiter fixture.
    fn me_and_atas() -> (String, serde_json::Value) {
        let v: serde_json::Value = serde_json::from_str(include_str!("testdata/jupiter-swap.json")).unwrap();
        (v["self_address"].as_str().unwrap().to_string(), v["action"]["atas"].clone())
    }
    fn setup() -> Extension {
        let (me, _) = me_and_atas();
        ctx(owner(), NearToken::ZERO, NOW);
        let mut c = Extension::new(owner(), signer(), DELAY);
        let mut g = grant();
        g.sol = Some(SolGrant { self_address: me, venues: vec![SolVenue { program: PHX.into(), scope: MARKET.into() }], recipients: vec![], nft_floor: U128(0), perps: vec![] });
        g.caps.push(Cap { asset: "sol".into(), daily: U128(1_000_000_000) });
        g.caps.push(Cap { asset: "spl:EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".into(), daily: U128(10_000_000) });
        ctx(wallet(), GRANT_DEPOSIT, NOW);
        c.grant_session("agent-1".into(), g);
        c
    }
    /// Swap with an IOC packet: side, no price limit, base and quote lots.
    fn swap(side: u8, base: u64, quote: u64, quote_account: Option<&str>) -> serde_json::Value {
        let (me, atas) = me_and_atas();
        let wsol_ata = sol::fmt(&sol::ata(&sol::b58(&me).unwrap(), &sol::b58("So11111111111111111111111111111111111111112").unwrap(), &sol::b58("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap(), atas[0]["bump"].as_u64().unwrap() as u8));
        let usdc_ata = sol::fmt(&sol::ata(&sol::b58(&me).unwrap(), &sol::b58("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap(), &sol::b58("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap(), atas[1]["bump"].as_u64().unwrap() as u8));
        let mut d = vec![0u8, 2, side, 0];
        d.extend(base.to_le_bytes());
        d.extend(quote.to_le_bytes());
        d.extend([0u8; 8 + 8]); // min fills
        d.extend([1u8, 0]); // self-trade behaviour, no match limit
        d.extend([0u8; 16]); // client order id
        d.extend([0u8, 0, 0]); // only deposited funds: no; no slot or time limit
        let acc = |k: &str, s: bool, w: bool| serde_json::json!({ "pubkey": k, "signer": s, "writable": w });
        let accounts = vec![
            acc(PHX, false, false), acc("7aDTsspkQNGKmrexAN7FLx9oxU3iPczSSvHNggyuqYkR", false, false), acc(MARKET, false, true), acc(&me, true, false),
            acc(&wsol_ata, false, true), acc(quote_account.unwrap_or(&usdc_ata), false, true),
            acc("8g4Z9d6PqGkgH31tMW6FwxGhwYJrXpxZHQrkikpLJKrG", false, true), acc("3HSYXeGc3LjEPCuzoNDjQN37F1ebsSiR4CqXVqQCdekZ", false, true),
            acc("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", false, false),
        ];
        serde_json::json!({ "type": "sol_tx", "recent_blockhash": "11111111111111111111111111111111", "atas": atas,
            "instructions": [{ "program": PHX, "accounts": accounts, "data": hex::encode(d) }] })
    }

    #[test]
    fn phoenix_rules() {
        let c = setup();
        let s = c.get_session(wallet(), "agent-1".into()).unwrap();
        let check = |a: serde_json::Value| rules::check(&s, &serde_json::from_value(a).unwrap(), Mode::Direct, NOW + 1000, &wallet(), &ext(), &signer()).map(|p| p.spends);
        // Sell 100 base lots (0.1 SOL), buy with 2 USDC; plus the 5,000-lamport fee.
        assert_eq!(check(swap(1, 100, 0, None)).unwrap(), vec![("sol".to_string(), 100_000_000 + 5_000)]);
        let spends = check(swap(0, 0, 2_000_000, None)).unwrap();
        assert_eq!(spends[0], ("spl:EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(), 2_000_000));
        // A buy budgeted in base lots, a sell in quote lots: refused.
        assert_eq!(check(swap(0, 50, 0, None)), Err("buy in quote lots, sell in base lots"));
        assert_eq!(check(swap(1, 0, 5, None)), Err("buy in quote lots, sell in base lots"));
        // Proceeds to someone else's account.
        assert_eq!(check(swap(1, 100, 0, Some("9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM"))), Err("venue call sends the result to someone else"));
        // Over the 10 USDC cap.
        assert_eq!(check(swap(0, 0, 20_000_000, None)), Err("over the daily cap"));
    }
}

/// Phoenix perps, from actions the demo agent built against a fork of
/// mainnet (agent/phoenix-perps.mjs, FIXTURE_DIR).
mod phoenix_perps {
    use super::*;
    use crate::perps::{PerpsDesk, PerpsMarket};
    use crate::rules::SolGrant;
    use near_sdk::json_types::U64;

    const SOL_PERP: &str = "71Si24E4uc3oCaPbPZTozC1ptSNNqygjjebxSmErSsC2";
    const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

    fn fixture(name: &str) -> (String, String, serde_json::Value) {
        let text = match name {
            "deposit" => include_str!("testdata/perps-deposit.json"),
            "open" => include_str!("testdata/perps-open.json"),
            "trade" => include_str!("testdata/perps-trade.json"),
            "close" => include_str!("testdata/perps-close.json"),
            "withdraw" => include_str!("testdata/perps-withdraw.json"),
            "too-big" => include_str!("testdata/perps-too-big.json"),
            "other-market" => include_str!("testdata/perps-other-market.json"),
            "no-limit" => include_str!("testdata/perps-no-limit.json"),
            "trader-withdraw" => include_str!("testdata/perps-trader-withdraw.json"),
            "authority-order" => include_str!("testdata/perps-authority-order.json"),
            "delegate-stranger" => include_str!("testdata/perps-delegate-stranger.json"),
            "withdraw-stranger" => include_str!("testdata/perps-withdraw-stranger.json"),
            _ => unreachable!(),
        };
        let v: serde_json::Value = serde_json::from_str(text).unwrap();
        (v["self_address"].as_str().unwrap().into(), v["trader_key"].as_str().unwrap().into(), v["action"].clone())
    }
    fn session(desk: bool) -> Session {
        let (me, key, _) = fixture("deposit");
        ctx(owner(), NearToken::ZERO, NOW);
        let mut c = Extension::new(owner(), signer(), DELAY);
        let mut g = grant();
        let perps = if desk { vec![PerpsDesk { sub: 1, trader_key: key, markets: vec![PerpsMarket { market: SOL_PERP.into(), max_base_lots: U64(1000) }] }] } else { vec![] };
        g.sol = Some(SolGrant { self_address: me, venues: vec![], recipients: vec![], nft_floor: U128(0), perps });
        g.caps.push(Cap { asset: "sol".into(), daily: U128(1_000_000_000) });
        g.caps.push(Cap { asset: format!("spl:{USDC}"), daily: U128(100_000_000) });
        g.caps.push(Cap { asset: format!("phoenix:{SOL_PERP}"), daily: U128(5_000) });
        ctx(wallet(), GRANT_DEPOSIT, NOW);
        c.grant_session("agent-1".into(), g);
        c.get_session(wallet(), "agent-1".into()).unwrap()
    }
    fn run(s: &Session, a: serde_json::Value) -> Result<rules::Plan, &'static str> {
        let act: Action = serde_json::from_value(a).unwrap();
        rules::check(s, &act, Mode::Direct, NOW + 1000, &wallet(), &ext(), &signer())
    }
    fn refused(s: &Session, a: serde_json::Value) -> &'static str {
        match run(s, a) {
            Err(e) => e,
            Ok(_) => panic!("passed"),
        }
    }
    fn spend(p: &rules::Plan, asset: &str) -> u128 {
        p.spends.iter().find(|(x, _)| x == asset).map(|x| x.1).unwrap_or(0)
    }

    #[test]
    fn the_desk_flow_passes() {
        let s = session(true);
        let p = run(&s, fixture("deposit").2).unwrap();
        assert_eq!(spend(&p, &format!("spl:{USDC}")), 40_000_000);
        let rules::Kind::Eddsa { path, .. } = &p.kind else { panic!() };
        assert_eq!(path, "solana");
        run(&s, fixture("open").2).unwrap();
        let p = run(&s, fixture("withdraw").2).unwrap();
        assert_eq!(spend(&p, &format!("spl:{USDC}")), 0);
        for t in ["trade", "close"] {
            let p = run(&s, fixture(t).2).unwrap();
            let rules::Kind::Eddsa { path, .. } = &p.kind else { panic!() };
            assert_eq!(path, "phoenix-1");
            assert_eq!(spend(&p, &format!("phoenix:{SOL_PERP}")), 50);
        }
    }

    #[test]
    fn no_desk_no_phoenix() {
        let s = session(false);
        assert_eq!(refused(&s, fixture("deposit").2), "not an allowed venue");
        assert_eq!(refused(&s, fixture("trade").2), "a subaccount the grant doesn't list");
    }

    #[test]
    fn refusals() {
        let s = session(true);
        for (f, why) in [
            ("too-big", "order larger than the grant allows"),
            ("other-market", "not an allowed market"),
            ("no-limit", "an order needs a limit price"),
            ("trader-withdraw", "only immediate-or-cancel orders"),
            ("authority-order", "orders go through the trading key"),
            ("delegate-stranger", "delegates only to the account's own trading key"),
            ("withdraw-stranger", "not the account's own token account"),
        ] {
            assert_eq!(refused(&s, fixture(f).2), why, "{f}");
        }
    }

    /// Index of the first instruction of `program` whose data starts with `prefix`.
    fn find(a: &serde_json::Value, program: &str, prefix: &str) -> usize {
        a["instructions"].as_array().unwrap().iter().position(|i| i["program"] == program && i["data"].as_str().unwrap().starts_with(prefix)).unwrap()
    }

    #[test]
    fn ember_approval_must_be_used_up() {
        let s = session(true);
        let mut a = fixture("withdraw").2;
        let i = find(&a, sol::TOKEN, "04");
        let mut d = hex::decode(a["instructions"][i]["data"].as_str().unwrap()).unwrap();
        d[1] = d[1].wrapping_add(1);
        a["instructions"][i]["data"] = serde_json::json!(hex::encode(d));
        assert_eq!(refused(&s, a), "an approval to Ember must be used up by an unwrap in the same transaction");
    }

    #[test]
    fn orders_only_on_the_granted_subaccount() {
        let s = session(true);
        // The trading key's order on the cross account (0) instead.
        let mut a = fixture("trade").2;
        a["sub"] = serde_json::json!(0);
        assert_eq!(refused(&s, a), "a subaccount the grant doesn't list");
        // A wrong bump: another trader account than the subaccount's.
        let mut a = fixture("trade").2;
        a["bump"] = serde_json::json!((a["bump"].as_u64().unwrap() + 1) % 256);
        assert_eq!(refused(&s, a), "an order on the subaccount the grant names, signed by its trading key");
    }

    #[test]
    fn subaccounts_hold_one_position() {
        let s = session(true);
        let (me, _, open) = fixture("open");
        // register_trader for subaccount 1: payer, authority, trader account, system program.
        let child = sol::fmt(&crate::perps::trader_account(&sol::b58(&me).unwrap(), 1, open["traders"][1]["bump"].as_u64().unwrap() as u8));
        let register = |max: u32| {
            let mut d = hex::decode("4bf3e0a701053320").unwrap();
            d.extend(max.to_le_bytes());
            d.extend([0, 0, 0, 0, 0, 1]);
            serde_json::json!({ "program": crate::perps::PROGRAM, "data": hex::encode(d), "accounts": [
                { "pubkey": crate::perps::PROGRAM, "signer": false, "writable": false },
                { "pubkey": crate::perps::LOG_AUTHORITY, "signer": false, "writable": false },
                { "pubkey": crate::perps::GLOBAL_CONFIG, "signer": false, "writable": false },
                { "pubkey": me, "signer": true, "writable": true },
                { "pubkey": me, "signer": false, "writable": false },
                { "pubkey": child, "signer": false, "writable": true },
                { "pubkey": sol::SYSTEM, "signer": false, "writable": false },
            ]})
        };
        let mut a = open.clone();
        a["instructions"] = serde_json::json!([register(1)]);
        assert_eq!(spend(&run(&s, a.clone()).unwrap(), "sol"), crate::perps::SUBACCOUNT_RENT + 5_000);
        a["instructions"] = serde_json::json!([register(128)]);
        assert_eq!(refused(&s, a), "only the account's own trader accounts");
    }
}
