use super::*;
use near_sdk::mock::MockAction;
use near_sdk::test_utils::{VMContextBuilder, get_created_receipts};
use near_sdk::testing_env;

const NAME: &str = "alice.marvii.testnet";
const WALLET: &str = "0s3465557e026f471c0e84898f799c8032f356c77a";

fn acct(s: &str) -> AccountId {
    s.parse().unwrap()
}

/// Sets the call context. Storage carries over between calls in a test.
fn call_as(current: &str, predecessor: &str, balance: NearToken, input: &str) {
    let mut b = VMContextBuilder::new();
    b.current_account_id(acct(current)).predecessor_account_id(acct(predecessor)).account_balance(balance).storage_usage(300);
    let mut c = b.build();
    c.input = std::rc::Rc::from(input.as_bytes());
    testing_env!(c);
}

fn refused(f: impl FnOnce() + std::panic::UnwindSafe) -> bool {
    std::panic::catch_unwind(f).is_err()
}

fn made() {
    call_as(NAME, "marvii.testnet", NearToken::from_millinear(10), &format!("{{\"target\":\"{WALLET}\"}}"));
    init();
}

/// The method and JSON arguments of a receipt's only function call.
fn call(r: &near_sdk::mock::Receipt) -> (String, String) {
    match &r.actions[..] {
        [MockAction::FunctionCallWeight { method_name, args, attached_deposit, .. }] => {
            assert_eq!(attached_deposit.as_yoctonear(), 1);
            (String::from_utf8(method_name.clone()).unwrap(), String::from_utf8(args.clone()).unwrap())
        }
        a => panic!("unexpected actions {a:?}"),
    }
}

#[test]
fn only_the_parent_registry_sets_the_target_once() {
    made();
    assert_eq!(target_account(), acct(WALLET));
    // Not again, not even by the registry.
    call_as(NAME, "marvii.testnet", NearToken::from_millinear(10), "{\"target\":\"mallory.testnet\"}");
    assert!(refused(|| init()));
    assert_eq!(target_account(), acct(WALLET));
}

#[test]
fn nobody_but_the_direct_parent_can_initialise() {
    for (me, caller) in [
        (NAME, "mallory.testnet"),
        (NAME, "testnet"),
        (NAME, "evil.marvii.testnet"),
        (NAME, NAME),
        ("a.alice.marvii.testnet", "marvii.testnet"),
        ("marvii.testnet", "marvii.testnet"),
        ("xmarvii.testnet", "marvii.testnet"),
    ] {
        call_as(me, caller, NearToken::from_millinear(10), "{\"target\":\"mallory.testnet\"}");
        assert!(refused(|| init()), "{me} by {caller}");
    }
}

#[test]
fn forward_sends_everything_above_storage_to_the_target_only() {
    made();
    call_as(NAME, "anyone.testnet", NearToken::from_near(1), "");
    let keep = env::storage_byte_cost().as_yoctonear() * 300 + RESERVE.as_yoctonear();
    assert_eq!(available(), NearToken::from_near(1).as_yoctonear() - keep);
    do_forward();
    let r = get_created_receipts();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].receiver_id, acct(WALLET));
    match &r[0].actions[..] {
        [MockAction::Transfer { deposit, .. }] => assert_eq!(deposit.as_yoctonear(), NearToken::from_near(1).as_yoctonear() - keep),
        a => panic!("unexpected actions {a:?}"),
    }
}

#[test]
fn nothing_to_forward_is_refused() {
    made();
    call_as(NAME, "anyone.testnet", NearToken::from_yoctonear(env::storage_byte_cost().as_yoctonear() * 300), "");
    assert_eq!(available(), 0);
    assert!(refused(|| do_forward()));
}

#[test]
fn tokens_go_to_the_target() {
    made();
    // ft_transfer_call: the token calls ft_on_transfer; a msg that names
    // someone else (even as a JSON key inside the string) changes nothing.
    call_as(NAME, "usdc.testnet", NearToken::from_millinear(10), r#"{"sender_id":"sender.testnet","amount":"500","msg":"{\"amount\":\"9\",\"receiver_id\":\"mallory.testnet\"}"}"#);
    on_ft_transfer();
    let r = get_created_receipts();
    assert_eq!(r[0].receiver_id, acct("usdc.testnet"));
    let (m, a) = call(&r[0]);
    assert_eq!(m, "ft_transfer");
    assert_eq!(a, format!("{{\"receiver_id\":\"{WALLET}\",\"amount\":\"500\",\"memo\":\"forwarded\"}}"));
    // forward_ft from anyone: the receiver is still the target, never the caller.
    call_as(NAME, "mallory.testnet", NearToken::from_millinear(10), r#"{ "amount" : "7", "token_id" : "usdc.testnet" }"#);
    do_forward_ft();
    let r = get_created_receipts();
    let (_, a) = call(&r[0]);
    assert!(r[0].receiver_id == acct("usdc.testnet") && a.contains(WALLET) && a.contains("\"7\"") && !a.contains("mallory"), "{a}");
}

#[test]
fn malformed_arguments_are_refused() {
    made();
    for bad in [
        r#"{"token_id":"usdc.testnet","amount":7}"#,
        r#"{"token_id":"usdc.testnet","amount":"-7"}"#,
        r#"{"token_id":"usdc.testnet","amount":"0x10"}"#,
        r#"{"token_id":"usdc.testnet","amount":"999999999999999999999999999999999999999999"}"#,
        r#"{"token_id":"usdc.testnet","amount":"1\"}"#,
        r#"{"token_id":"Not An Account","amount":"7"}"#,
        r#"{"token_id":"usdc.testnet"}"#,
        r#"["usdc.testnet","7"]"#,
        "",
    ] {
        call_as(NAME, "anyone.testnet", NearToken::from_millinear(10), bad);
        assert!(refused(|| do_forward_ft()), "{bad}");
    }
}

#[test]
fn find_field_reads_only_top_level_keys() {
    let j = br#"{"a":{"amount":"1"},"list":[1,"amount",{"x":2}],"n":12,"t":true,"amount":"42"}"#;
    assert_eq!(find_field(j, "amount").as_deref(), Some("42"));
    assert_eq!(find_field(br#"{"msg":"\"amount\":\"9\""}"#, "amount"), None);
    assert_eq!(find_field(br#"{"amount":"4\u0032"}"#, "amount"), None, "escapes refused");
}

#[test]
fn amounts_parse_up_to_u128_max() {
    for v in [0u128, 7, 10, 1_000_000_000_000_000_000_000_000, u128::MAX] {
        assert_eq!(number(&v.to_string()), v);
    }
    assert!(refused(|| { number("340282366920938463463374607431768211456"); }), "u128::MAX + 1");
}
