//! A name's deposit address, e.g. `alice.marvii.testnet`.
//!
//! The registry creates the account with this code, no access keys, and one
//! wallet as its target. Whatever arrives is forwarded to that wallet:
//! - NEAR, by `forward`, which anyone may call (the relayer calls it once a
//!   deposit lands). A plain transfer runs no code, so NEAR waits here until then.
//! - Tokens sent with `ft_transfer_call`, at once in `ft_on_transfer`;
//!   tokens sent with a plain `ft_transfer`, by `forward_ft`.
//!
//! The target is set once, by the parent registry, and can never change:
//! there are no keys, no owner and no other way to move funds.
//!
//! Written against near-sdk's low-level `env` without its macros or JSON
//! library, because this code is published as a global contract, which
//! burns NEAR per byte. The argument parsing is deliberately narrow: each
//! method reads only the string fields it needs.

use near_sdk::{AccountId, Gas, NearToken, env};

/// Kept on top of the storage the account uses, for the 1 yoctoNEAR each
/// token transfer attaches.
pub const RESERVE: NearToken = NearToken::from_millinear(1);
const FT_TRANSFER_GAS: Gas = Gas::from_tgas(10);
const TARGET_KEY: &[u8] = b"t";

/// Called once, by the parent registry, in the batch that creates the account.
pub fn init() {
    if env::storage_has_key(TARGET_KEY) {
        env::panic_str("already initialised");
    }
    let parent = env::predecessor_account_id();
    let me = env::current_account_id();
    let ok = me.as_str().strip_suffix(parent.as_str()).is_some_and(|n| n.len() > 1 && n.ends_with('.') && !n[..n.len() - 1].contains('.'));
    if !ok {
        env::panic_str("only the parent registry sets the target");
    }
    let target = account(&field("target"));
    env::storage_write(TARGET_KEY, target.as_bytes());
}

/// View: the wallet everything goes to, as a JSON string.
pub fn view_target() {
    return_json_string(target_account().as_str());
}

/// View: yoctoNEAR waiting here above what the account must keep, as a JSON string.
pub fn view_pending() {
    return_json_string(&available().to_string());
}

/// Sends the NEAR waiting here to the wallet. Anyone may call it.
pub fn do_forward() {
    let amount = available();
    if amount == 0 {
        env::panic_str("nothing to forward");
    }
    let p = env::promise_batch_create(&target_account());
    env::promise_batch_action_transfer(p, NearToken::from_yoctonear(amount));
}

/// Sends `amount` of the token `token_id` held here to the wallet. Anyone
/// may call it; the wallet must be registered with the token.
pub fn do_forward_ft() {
    let token = account(&field("token_id"));
    let amount = number(&field("amount"));
    if amount == 0 {
        env::panic_str("nothing to forward");
    }
    ft_transfer(&token, amount);
}

/// NEP-141: tokens sent with `ft_transfer_call` go straight on to the
/// wallet. All are kept (returns "0"), then transferred.
pub fn on_ft_transfer() {
    let amount = number(&field("amount"));
    if amount > 0 {
        ft_transfer(&env::predecessor_account_id(), amount);
    }
    return_json_string("0");
}

fn target_account() -> AccountId {
    let raw = env::storage_read(TARGET_KEY).unwrap_or_else(|| env::panic_str("not initialised"));
    account(core::str::from_utf8(&raw).unwrap_or_else(|_| env::panic_str("bad state")))
}

fn available() -> u128 {
    let keep = env::storage_byte_cost().as_yoctonear().saturating_mul(env::storage_usage() as u128).saturating_add(RESERVE.as_yoctonear());
    env::account_balance().as_yoctonear().saturating_sub(keep)
}

fn ft_transfer(token: &AccountId, amount: u128) {
    let target = target_account();
    let mut args = String::from("{\"receiver_id\":\"");
    args.push_str(target.as_str());
    args.push_str("\",\"amount\":\"");
    args.push_str(&amount.to_string());
    args.push_str("\",\"memo\":\"forwarded\"}");
    let p = env::promise_batch_create(token);
    env::promise_batch_action_function_call(p, "ft_transfer", args.as_bytes(), NearToken::from_yoctonear(1), FT_TRANSFER_GAS);
}

fn return_json_string(s: &str) {
    let mut out = Vec::with_capacity(s.len() + 2);
    out.push(b'"');
    out.extend_from_slice(s.as_bytes());
    out.push(b'"');
    env::value_return(&out);
}

/// The value of the top-level string field `key` in the call's JSON
/// arguments. The value must be a plain string without escapes (account IDs
/// and decimal amounts are); anything else is refused.
pub(crate) fn field(key: &str) -> String {
    let input = env::input().unwrap_or_default();
    match find_field(&input, key) {
        Some(v) => v,
        None => env::panic_str("missing or malformed argument"),
    }
}

/// Finds `"key"` as an object key at the top level of `json` and returns its
/// string value. Walks the object token by token, so a key's name inside
/// another string (e.g. inside `msg`) never matches.
pub(crate) fn find_field(json: &[u8], key: &str) -> Option<String> {
    let mut i = skip_ws(json, 0);
    if json.get(i) != Some(&b'{') {
        return None;
    }
    i += 1;
    loop {
        i = skip_ws(json, i);
        if json.get(i) == Some(&b'}') {
            return None;
        }
        let (k, next) = read_string(json, i)?;
        i = skip_ws(json, next);
        if json.get(i) != Some(&b':') {
            return None;
        }
        i = skip_ws(json, i + 1);
        if json.get(i) == Some(&b'"') {
            let (v, next) = read_string(json, i)?;
            if k == key {
                return if v.contains('\\') { None } else { Some(v) };
            }
            i = next;
        } else if k == key {
            return None; // not a string
        } else {
            i = skip_value(json, i)?;
        }
        i = skip_ws(json, i);
        match json.get(i) {
            Some(b',') => i += 1,
            _ => return None,
        }
    }
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while matches!(b.get(i), Some(b' ' | b'\n' | b'\r' | b'\t')) {
        i += 1;
    }
    i
}

/// Reads a JSON string starting at `b[i] == '"'`; returns its raw contents
/// (escapes left as written) and the index after the closing quote.
fn read_string(b: &[u8], i: usize) -> Option<(String, usize)> {
    if b.get(i) != Some(&b'"') {
        return None;
    }
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            b'"' => return Some((String::from_utf8(b[i + 1..j].to_vec()).ok()?, j + 1)),
            _ => j += 1,
        }
    }
    None
}

/// Skips a non-string JSON value (number, literal, object or array).
fn skip_value(b: &[u8], mut i: usize) -> Option<usize> {
    let mut depth = 0usize;
    while i < b.len() {
        match b[i] {
            b'"' => i = read_string(b, i)?.1,
            b'{' | b'[' => {
                depth += 1;
                i += 1;
            }
            b'}' | b']' => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
                i += 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            b',' if depth == 0 => return Some(i),
            _ => i += 1,
        }
    }
    None
}

fn account(s: &str) -> AccountId {
    s.parse().unwrap_or_else(|_| env::panic_str("invalid account"))
}

fn number(s: &str) -> u128 {
    if s.is_empty() || s.len() > 39 || !s.bytes().all(|c| c.is_ascii_digit()) {
        env::panic_str("invalid amount");
    }
    s.parse().unwrap_or_else(|_| env::panic_str("invalid amount"))
}

/// The exported entry points; the logic is in plain functions above so the
/// unit tests can call it (an `extern "C"` function can't unwind a panic).
#[cfg(target_arch = "wasm32")]
mod exports {
    #[unsafe(no_mangle)]
    pub extern "C" fn new() {
        super::init()
    }
    #[unsafe(no_mangle)]
    pub extern "C" fn target() {
        super::view_target()
    }
    #[unsafe(no_mangle)]
    pub extern "C" fn pending() {
        super::view_pending()
    }
    #[unsafe(no_mangle)]
    pub extern "C" fn forward() {
        super::do_forward()
    }
    #[unsafe(no_mangle)]
    pub extern "C" fn forward_ft() {
        super::do_forward_ft()
    }
    #[unsafe(no_mangle)]
    pub extern "C" fn ft_on_transfer() {
        super::on_ft_transfer()
    }
}

#[cfg(test)]
mod tests;
