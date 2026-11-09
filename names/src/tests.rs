use super::*;
use near_sdk::mock::MockAction;
use near_sdk::test_utils::{VMContextBuilder, get_created_receipts};
use near_sdk::{PromiseResult, RuntimeFeesConfig, testing_env, test_vm_config};

const REG: &str = "marvii.testnet";
const W1: &str = "0s3465557e026f471c0e84898f799c8032f356c77a";
const W2: &str = "0sf6db5e61bfb693d59e4596afb8ea4381f80a408d";
const HASH: [u8; 32] = [7; 32];

fn acct(s: &str) -> AccountId {
    s.parse().unwrap()
}

fn as_caller(predecessor: &str) {
    let mut b = VMContextBuilder::new();
    b.current_account_id(acct(REG)).predecessor_account_id(acct(predecessor)).account_balance(NearToken::from_near(5));
    testing_env!(b.build());
}

fn registry() -> Names {
    as_caller(REG);
    Names::new(HASH.into())
}

fn refused(f: impl FnOnce() + std::panic::UnwindSafe) -> bool {
    std::panic::catch_unwind(f).is_err()
}

#[test]
fn register_creates_a_keyless_forwarder_to_the_caller() {
    let mut n = registry();
    as_caller(W1);
    let _ = n.register("alice".into());
    let r = get_created_receipts();
    assert_eq!(r[0].receiver_id, acct("alice.marvii.testnet"));
    let mut saw = (false, false, false, false);
    for a in &r[0].actions {
        match a {
            MockAction::CreateAccount { .. } => saw.0 = true,
            MockAction::Transfer { deposit, .. } => saw.1 = *deposit == NAME_FUNDING,
            MockAction::UseGlobalContract { contract_id, .. } => {
                // The SDK doesn't export the identifier type; its Debug form shows the hash in base58.
                let d = format!("{contract_id:?}");
                saw.2 = d.starts_with("CodeHash") && d.contains(&near_sdk::bs58::encode(HASH).into_string())
            }
            MockAction::FunctionCallWeight { method_name, args, .. } => {
                saw.3 = method_name == b"new" && String::from_utf8(args.clone()).unwrap() == format!("{{\"target\":\"{W1}\"}}")
            }
            MockAction::AddKeyWithFullAccess { .. } | MockAction::AddKeyWithFunctionCall { .. } => panic!("the name's account must have no keys"),
            other => panic!("unexpected action {other:?}"),
        }
    }
    assert_eq!(saw, (true, true, true, true));
    // Until the callback confirms, the name is held (so no one else can take it).
    assert_eq!(n.wallet_of("alice".into()), Some(acct(W1)));
    assert_eq!(n.name_of(acct(W1)), Some(acct("alice.marvii.testnet")));
    assert_eq!(n.wallet_of("alice.marvii.testnet".into()), Some(acct(W1)));
}

#[test]
fn one_name_per_wallet_and_names_are_never_reassigned() {
    let mut n = registry();
    as_caller(W1);
    let _ = n.register("alice".into());
    let mut n2 = std::panic::AssertUnwindSafe(&mut n);
    assert!(refused(move || drop(n2.register("bob".into()))), "second name for the same wallet");
    as_caller(W2);
    let mut n3 = std::panic::AssertUnwindSafe(&mut n);
    assert!(refused(move || drop(n3.register("alice".into()))), "a taken name");
}

#[test]
fn only_marvii_wallets_register() {
    let mut n = registry();
    for caller in ["mallory.testnet", "0x1111111111111111111111111111111111111111", REG] {
        as_caller(caller);
        let mut m = std::panic::AssertUnwindSafe(&mut n);
        assert!(refused(move || drop(m.register("carol".into()))), "{caller}");
    }
}

#[test]
fn bad_and_reserved_names_are_refused() {
    let mut n = registry();
    as_caller(W1);
    for bad in ["a", "Alice", "al.ice", "-alice", "alice-", "al--ice", "al_-ice", "al ice", "marvii", "admin", "0xabc", "0s12", &"a".repeat(33), "ali€e", ""] {
        let mut m = std::panic::AssertUnwindSafe(&mut n);
        let b = bad.to_string();
        assert!(refused(move || drop(m.register(b))), "{bad:?}");
    }
    for (i, good) in ["al", "alice-1", "a_b-c", "satoshi99"].iter().enumerate() {
        let w = format!("0s{:040x}", i + 1);
        as_caller(&w);
        let _ = n.register(good.to_string());
        assert_eq!(n.name_of(acct(&w)).unwrap().as_str(), format!("{good}.marvii.testnet"));
    }
}

fn callback(result: PromiseResult, predecessor: &str) {
    let mut b = VMContextBuilder::new();
    b.current_account_id(acct(REG)).predecessor_account_id(acct(predecessor));
    testing_env!(b.build(), test_vm_config(), RuntimeFeesConfig::test(), Default::default(), vec![result]);
}

#[test]
fn a_failed_creation_frees_the_name_and_a_success_lists_it() {
    let mut n = registry();
    as_caller(W1);
    let _ = n.register("alice".into());
    callback(PromiseResult::Failed, REG);
    assert!(!n.on_created("alice".into(), acct(W1), Err(PromiseError::Failed)));
    assert_eq!(n.wallet_of("alice".into()), None);
    assert_eq!(n.name_of(acct(W1)), None);
    assert_eq!(n.count(), 0);
    // Registering again works, and success lists it.
    as_caller(W1);
    let _ = n.register("alice".into());
    callback(PromiseResult::Successful(vec![]), REG);
    assert!(n.on_created("alice".into(), acct(W1), Ok(())));
    assert_eq!(n.names(0, 10), vec![acct("alice.marvii.testnet")]);
}

#[test]
fn only_the_registry_runs_its_callback() {
    let mut n = registry();
    as_caller(W1);
    let _ = n.register("alice".into());
    callback(PromiseResult::Failed, W1);
    let mut m = std::panic::AssertUnwindSafe(&mut n);
    assert!(refused(move || { let _ = m.on_created("alice".into(), acct(W1), Err(PromiseError::Failed)); }));
    assert_eq!(n.wallet_of("alice".into()), Some(acct(W1)), "the name stays");
}
