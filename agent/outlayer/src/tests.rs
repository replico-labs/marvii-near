use super::*;

const EXT: &str = "d591504aa95d1d7164f11dfac2016363dd18788458040c7fc3ee32cd4344e0ec";
const WALLET: &str = "0s3465557e026f471c0e84898f799c8032f356c77a";

fn order() -> Action {
    Action::HlOrder { asset: 4, is_buy: true, price: "2100.5".into(), size: "0.01".into(), reduce_only: false, tif: "Gtc".into(), nonce: 1762700000000 }
}

#[test]
fn message_matches_the_node_agent() {
    // agent.mjs: JSON.stringify({ domain, extension, wallet, key_id, nonce, deadline_ms, action })
    let (msg, digest) = build_message(EXT, WALLET, "tee-agent", 8, 1_762_700_000_000, &order()).unwrap();
    assert_eq!(
        msg,
        format!(r#"{{"domain":"marvii-session-v1","extension":"{EXT}","wallet":"{WALLET}","key_id":"tee-agent","nonce":8,"deadline_ms":1762700300000,"action":{{"type":"hl_order","asset":4,"is_buy":true,"price":"2100.5","size":"0.01","reduce_only":false,"tif":"Gtc","nonce":1762700000000}}}}"#)
    );
    assert_eq!(digest, <[u8; 32]>::from(Sha256::digest(msg.as_bytes())));
}

#[test]
fn other_actions_and_fields_are_refused() {
    let bad: Result<Input, _> = serde_json::from_str(r#"{"operation":"sign","action":{"type":"evm_call","to":"0x0"}}"#);
    assert!(bad.is_err());
    let extra: Result<Input, _> = serde_json::from_str(r#"{"operation":"sign","message_hex":"00"}"#);
    assert!(extra.is_err());
    let extra_field: Result<Action, _> = serde_json::from_str(r#"{"type":"hl_cancel","asset":4,"oid":1,"nonce":1,"vault":"x"}"#);
    assert!(extra_field.is_err());
}

#[test]
fn inputs_are_checked() {
    let b = 1_762_700_000_000;
    assert!(build_message("Bad\"ext", WALLET, "k", 1, b, &order()).is_err());
    assert!(build_message(EXT, "a..b", "k", 1, b, &order()).is_err());
    assert!(build_message(EXT, WALLET, "k\"", 1, b, &order()).is_err());
    let mut o = order();
    if let Action::HlOrder { price, .. } = &mut o { *price = "1e9".into() }
    assert!(build_message(EXT, WALLET, "k", 1, b, &o).is_err());
    let lev = Action::HlLeverage { asset: 4, is_cross: true, leverage: 0, nonce: 1 };
    assert!(build_message(EXT, WALLET, "k", 1, b, &lev).is_err());
    let cancel = Action::HlCancel { asset: 4, oid: 9, nonce: 1 };
    assert!(build_message(EXT, WALLET, "tee-agent", 1, b, &cancel).is_ok());
}
