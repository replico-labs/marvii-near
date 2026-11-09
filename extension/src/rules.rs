//! The rules a session key works within. Pure functions, so every rule can
//! be tested on its own.

use crate::evm::{self, Decoded, Eip1559};
use crate::hl;
use crate::utxo;
use near_sdk::{AccountId, NearToken, json_types::U128, near};

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NearVenue {
    pub receiver_id: AccountId,
    pub method: String,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvmVenue {
    pub chain_id: u64,
    /// 0x-prefixed contract address.
    pub to: String,
    /// 0x-prefixed 4-byte function selector.
    pub selector: String,
}

/// A Hyperliquid perpetual the agent may trade, by Hyperliquid's asset
/// index (testnet: 4 is ETH), and the most leverage it may set.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HlMarket {
    pub asset: u32,
    pub max_leverage: u32,
}

/// Payments on a payment-only chain (Bitcoin, Litecoin, Bitcoin Cash,
/// Zcash): the account's own output script (for change) and the scripts the
/// agent may pay, all hex. Caps use the asset `utxo:<chain>`, counted as
/// everything that leaves the account, fee included.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UtxoGrant {
    pub chain: String,
    pub self_script: String,
    pub recipients: Vec<String>,
}

/// The cap asset for Hyperliquid orders: their value, price × size, in
/// micro-USD (6 decimals), counted whether they open or reduce.
pub const HL_CAP_ASSET: &str = "hyperliquid:usd";

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cap {
    /// `near`, `nep141:<token account>`, `evm:<chain id>` (native coin) or
    /// `evm:<chain id>:<0x token address>`, lowercase.
    pub asset: String,
    /// The most that may leave the wallet per UTC day, in the asset's
    /// smallest unit.
    pub daily: U128,
}

/// What the passkey granted.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    /// The agent's ed25519 public key, 32 bytes hex.
    pub public_key: String,
    pub label: String,
    pub expires_at_ms: u64,
    /// The key stops working after this long unused.
    pub inactivity_ms: u64,
    pub near_venues: Vec<NearVenue>,
    pub evm_venues: Vec<EvmVenue>,
    /// NEAR accounts that NEP-141 `ft_transfer` may pay.
    pub near_recipients: Vec<AccountId>,
    /// `<chain id>:<0x address>` that ERC-20 `transfer` and native
    /// transfers may pay.
    pub evm_recipients: Vec<String>,
    pub caps: Vec<Cap>,
    /// Whether the agent may propose payments to new recipients and calls to
    /// new contracts. They wait in a queue (the extension's delay) where the
    /// passkey can cancel them; caps and expiry still apply.
    #[serde(default)]
    pub queue_new_targets: bool,
    /// Hyperliquid (testnet) markets the agent may trade. Orders count
    /// against the `hyperliquid:usd` cap.
    #[serde(default)]
    pub hl_markets: Vec<HlMarket>,
    /// The account's own EVM address (from Chain Signatures, path `evm`),
    /// as the passkey saw it. Venue calls with a known layout (Aave supply,
    /// withdraw, repay; Uniswap swaps) must send their result here.
    #[serde(default)]
    pub evm_self: Option<String>,
    /// Payment-only chains the agent may pay on.
    #[serde(default)]
    pub utxo: Vec<UtxoGrant>,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub grant: Grant,
    pub nonce: u64,
    pub last_used_ms: u64,
    pub revoked: bool,
    /// The UTC day `spent` refers to.
    pub day: u64,
    pub spent: Vec<(String, U128)>,
}

impl Session {
    pub fn new(grant: Grant, nonce: u64, now: u64) -> Self {
        Self { grant, nonce, last_used_ms: now, revoked: false, day: now / DAY_MS, spent: vec![] }
    }

    pub fn public_key(&self) -> [u8; 32] {
        // Checked when granted.
        hex::decode(&self.grant.public_key).unwrap().try_into().unwrap()
    }

    pub fn spent_on(&self, asset: &str, now: u64) -> u128 {
        if self.day != now / DAY_MS {
            return 0;
        }
        self.spent.iter().find(|(a, _)| a == asset).map(|(_, v)| v.0).unwrap_or(0)
    }

    pub fn record(&mut self, spends: &[(String, u128)], now: u64) {
        let today = now / DAY_MS;
        if self.day != today {
            self.day = today;
            self.spent.clear();
        }
        for (asset, amount) in spends {
            match self.spent.iter_mut().find(|(a, _)| a == asset) {
                Some((_, v)) => v.0 += amount,
                None => self.spent.push((asset.clone(), U128(*amount))),
            }
        }
    }
}

/// The action an agent asks for.
#[near(serializers = [json])]
#[derive(Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// A call from the wallet to a NEAR contract.
    NearCall { receiver_id: AccountId, method: String, args: String, deposit: U128 },
    /// An EVM transaction, signed through Chain Signatures.
    EvmTx { tx: EvmTx },
    /// A Hyperliquid limit order. `price` and `size` as Hyperliquid writes
    /// them; `nonce` is the order's timestamp in milliseconds.
    HlOrder { asset: u32, is_buy: bool, price: String, size: String, reduce_only: bool, tif: String, nonce: u64 },
    /// Cancels a Hyperliquid order by its ID.
    HlCancel { asset: u32, oid: u64, nonce: u64 },
    /// Sets the leverage on a Hyperliquid market.
    HlLeverage { asset: u32, is_cross: bool, leverage: u32, nonce: u64 },
    /// A payment on a payment-only chain: the coins spent and every output.
    /// Zcash also takes its consensus branch ID and expiry height.
    UtxoPay {
        chain: String,
        inputs: Vec<UtxoIn>,
        outputs: Vec<UtxoOut>,
        #[serde(default)]
        branch_id: u32,
        #[serde(default)]
        expiry_height: u32,
    },
}

#[near(serializers = [json])]
#[derive(Clone, Debug)]
pub struct UtxoIn {
    /// As explorers show it, hex.
    pub txid: String,
    pub vout: u32,
    pub value: u64,
}

#[near(serializers = [json])]
#[derive(Clone, Debug)]
pub struct UtxoOut {
    /// The output script, hex.
    pub script: String,
    pub value: u64,
}

/// How far a Hyperliquid nonce (a timestamp) may be from the block time:
/// a signature is only good for an action meant to be sent now.
pub const HL_NONCE_WINDOW_MS: u64 = 5 * 60 * 1000;

#[near(serializers = [json])]
#[derive(Clone, Debug)]
pub struct EvmTx {
    pub chain_id: u64,
    pub nonce: u64,
    pub max_priority_fee_per_gas: U128,
    pub max_fee_per_gas: U128,
    pub gas_limit: u64,
    pub to: String,
    pub value: U128,
    /// 0x-prefixed call data ("0x" for none).
    pub data: String,
}

impl EvmTx {
    pub fn parse(&self) -> Result<Eip1559, &'static str> {
        Ok(Eip1559 {
            chain_id: self.chain_id,
            nonce: self.nonce,
            max_priority_fee_per_gas: self.max_priority_fee_per_gas.0,
            max_fee_per_gas: self.max_fee_per_gas.0,
            gas_limit: self.gas_limit,
            to: evm::parse_address(&self.to)?,
            value: self.value.0,
            data: evm::parse_hex(&self.data)?,
        })
    }
}

pub enum Kind {
    Near { receiver_id: AccountId, method: String, args: Vec<u8>, deposit: NearToken },
    Evm { path: String, hash: [u8; 32] },
    /// One signature per input, on the chain's own path.
    Utxo { path: String, hashes: Vec<[u8; 32]> },
}

/// `Direct`: everything must be on the grant's lists. `Queued`: recipients
/// and venues may be new (the delay and the passkey's cancel cover them);
/// every other rule still applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Direct,
    Queued,
}

pub struct Plan {
    pub kind: Kind,
    pub spends: Vec<(String, u128)>,
}

/// The Chain Signatures derivation path for every EVM chain: one address
/// on all of them, as users expect. Cross-chain replay is blocked by the
/// chain ID inside each EIP-1559 transaction, which is part of the hash the
/// extension builds; the extension never signs bare messages.
pub const EVM_PATH: &str = "evm";

/// Checks a grant before it's stored.
pub fn validate_grant(
    g: &Grant,
    key_id: &str,
    now: u64,
    wallet: &AccountId,
    extension: &AccountId,
    signer: &AccountId,
) -> Result<(), &'static str> {
    if key_id.is_empty() || key_id.len() > 64 {
        return Err("key_id must be 1 to 64 characters");
    }
    let pk = hex::decode(&g.public_key).map_err(|_| "public_key must be hex")?;
    if pk.len() != 32 {
        return Err("public_key must be 32 bytes");
    }
    if g.expires_at_ms <= now || g.expires_at_ms > now + crate::MAX_GRANT_MS {
        return Err("expiry must be in the future and within 90 days");
    }
    if g.inactivity_ms == 0 {
        return Err("inactivity_ms must be set");
    }
    for v in &g.near_venues {
        forbidden_receiver(&v.receiver_id, wallet, extension, signer)?;
    }
    for v in &g.evm_venues {
        evm::parse_address(&v.to)?;
        let sel = evm::parse_hex(&v.selector)?;
        if sel.len() != 4 {
            return Err("selector must be 4 bytes");
        }
        // Token movements go through the caps and recipient lists, not venues.
        if sel == evm::SEL_TRANSFER || sel == evm::SEL_APPROVE {
            return Err("transfer and approve can't be venues; use caps and recipients");
        }
    }
    for r in &g.evm_recipients {
        let (chain, addr) = r.split_once(':').ok_or("evm recipient must be <chain id>:<address>")?;
        chain.parse::<u64>().map_err(|_| "bad chain id in evm recipient")?;
        evm::parse_address(addr)?;
    }
    if let Some(a) = &g.evm_self {
        evm::parse_address(a)?;
    }
    for (i, u) in g.utxo.iter().enumerate() {
        if !utxo::CHAINS.contains(&u.chain.as_str()) {
            return Err("agents can pay only on Bitcoin, Litecoin, Bitcoin Cash and Zcash");
        }
        if g.utxo[..i].iter().any(|x| x.chain == u.chain) {
            return Err("a chain is listed twice");
        }
        let own = hex::decode(&u.self_script).map_err(|_| "scripts must be hex")?;
        if !utxo::self_script_ok(&u.chain, &own) {
            return Err("the account's own script has the wrong shape for that chain");
        }
        if u.recipients.len() > 16 {
            return Err("too many recipients");
        }
        for r in &u.recipients {
            let s = hex::decode(r).map_err(|_| "scripts must be hex")?;
            if !utxo::recipient_script_ok(&u.chain, &s) || r != &r.to_lowercase() {
                return Err("a recipient script isn't a standard lowercase script for that chain");
            }
        }
    }
    for (i, m) in g.hl_markets.iter().enumerate() {
        if m.max_leverage == 0 || m.max_leverage > 50 {
            return Err("max leverage must be 1 to 50");
        }
        if g.hl_markets[..i].iter().any(|x| x.asset == m.asset) {
            return Err("a Hyperliquid market is listed twice");
        }
    }
    for c in &g.caps {
        if c.asset != c.asset.to_lowercase() {
            return Err("cap assets must be lowercase");
        }
    }
    if g.near_venues.len() + g.evm_venues.len() + g.near_recipients.len() + g.evm_recipients.len() + g.caps.len() + g.hl_markets.len() + g.utxo.len() > 64 {
        return Err("grant too large");
    }
    Ok(())
}

/// Venues may never be the wallet, this extension (that would let a session
/// grant itself more) or the MPC signer (signing goes only through EvmTx,
/// where the path and hash are ours).
fn forbidden_receiver(r: &AccountId, wallet: &AccountId, extension: &AccountId, signer: &AccountId) -> Result<(), &'static str> {
    if r == wallet || r == extension || r == signer {
        return Err("venue can't be the wallet, the extension or the signer");
    }
    Ok(())
}

fn cap_for(g: &Grant, asset: &str) -> Option<u128> {
    g.caps.iter().find(|c| c.asset == asset).map(|c| c.daily.0)
}

/// Checks an action against the session. On success, returns what to send
/// and what it spends.
pub fn check(
    s: &Session,
    action: &Action,
    mode: Mode,
    now: u64,
    wallet: &AccountId,
    extension: &AccountId,
    signer: &AccountId,
) -> Result<Plan, &'static str> {
    if s.revoked {
        return Err("session revoked");
    }
    if now >= s.grant.expires_at_ms {
        return Err("session expired");
    }
    if now >= s.last_used_ms.saturating_add(s.grant.inactivity_ms) {
        return Err("session expired from inactivity");
    }
    let g = &s.grant;
    if mode == Mode::Queued && !g.queue_new_targets {
        return Err("this session can't queue new recipients or venues");
    }
    let queued = mode == Mode::Queued;
    let plan = match action {
        Action::NearCall { receiver_id, method, args, deposit } => {
            forbidden_receiver(receiver_id, wallet, extension, signer)?;
            let mut spends = Vec::new();
            // 1 yoctoNEAR is the usual security deposit (ft_transfer needs it),
            // not a payment.
            if deposit.0 > 1 {
                spends.push(("near".to_string(), deposit.0));
            }
            if method == "ft_transfer" || method == "ft_transfer_call" {
                let a: FtArgs = near_sdk::serde_json::from_str(args).map_err(|_| "bad ft args")?;
                let ok = if method == "ft_transfer" {
                    g.near_recipients.contains(&a.receiver_id)
                } else {
                    // ft_transfer_call hands tokens to a contract: only venues.
                    g.near_venues.iter().any(|v| v.receiver_id == a.receiver_id)
                };
                if !ok && !queued {
                    return Err("token recipient not allowed");
                }
                spends.push((format!("nep141:{receiver_id}"), a.amount.0));
            } else if !queued && !g.near_venues.iter().any(|v| &v.receiver_id == receiver_id && &v.method == method) {
                return Err("not an allowed venue");
            }
            Plan {
                kind: Kind::Near {
                    receiver_id: receiver_id.clone(),
                    method: method.clone(),
                    args: args.as_bytes().to_vec(),
                    deposit: NearToken::from_yoctonear(deposit.0),
                },
                spends,
            }
        }
        Action::EvmTx { tx } => {
            let t = tx.parse()?;
            let to = evm::fmt_address(&t.to);
            let chain = t.chain_id;
            let mut spends = Vec::new();
            if t.value > 0 {
                spends.push((format!("evm:{chain}"), t.value));
            }
            match evm::decode(&t.data)? {
                Decoded::Native => {
                    if !queued && !g.evm_recipients.iter().any(|r| r.to_lowercase() == format!("{chain}:{to}")) {
                        return Err("recipient not allowed");
                    }
                }
                Decoded::Transfer { recipient, amount } => {
                    if t.value > 0 {
                        return Err("token transfer with value");
                    }
                    let r = format!("{chain}:{}", evm::fmt_address(&recipient));
                    if !queued && !g.evm_recipients.iter().any(|x| x.to_lowercase() == r) {
                        return Err("recipient not allowed");
                    }
                    spends.push((format!("evm:{chain}:{to}"), amount));
                }
                Decoded::Approve { spender, amount } => {
                    if t.value > 0 {
                        return Err("approve with value");
                    }
                    let sp = evm::fmt_address(&spender);
                    // Approve only a venue, and count the whole allowance as spent.
                    if !queued && !g.evm_venues.iter().any(|v| v.chain_id == chain && v.to.to_lowercase() == sp) {
                        return Err("spender is not a venue");
                    }
                    spends.push((format!("evm:{chain}:{to}"), amount));
                }
                Decoded::Call { selector } => {
                    let sel = format!("0x{}", hex::encode(selector));
                    if !queued && !g.evm_venues.iter().any(|v| v.chain_id == chain && v.to.to_lowercase() == to && v.selector.to_lowercase() == sel) {
                        return Err("not an allowed venue");
                    }
                    // Where a venue call's result goes, for layouts we know:
                    // always the account itself, queued or not.
                    if let Some(b) = evm::beneficiary(&t.data)? {
                        let own = g.evm_self.as_deref().ok_or("this grant doesn't name the account's EVM address")?;
                        if b != evm::parse_address(own)? {
                            return Err("venue call sends the result to someone else");
                        }
                    }
                }
            }
            Plan { kind: Kind::Evm { path: EVM_PATH.to_string(), hash: t.signing_hash() }, spends }
        }
        Action::UtxoPay { chain, inputs, outputs, branch_id, expiry_height } => {
            let u = g.utxo.iter().find(|u| &u.chain == chain).ok_or("payments on this chain not allowed")?;
            if inputs.is_empty() || inputs.len() > utxo::MAX_INPUTS || outputs.is_empty() || outputs.len() > utxo::MAX_OUTPUTS {
                return Err("1 to 2 inputs and 1 to 3 outputs");
            }
            if chain == "zcash" && (*branch_id == 0 || *expiry_height == 0) {
                return Err("Zcash needs a branch ID and expiry height");
            }
            let own = hex::decode(&u.self_script).unwrap();
            let mut ins = vec![];
            let mut total_in: u64 = 0;
            for i in inputs {
                let txid: [u8; 32] = hex::decode(&i.txid).ok().and_then(|v| v.try_into().ok()).ok_or("txid must be 32 bytes hex")?;
                if ins.iter().any(|x: &utxo::Input| x.txid == txid && x.vout == i.vout) {
                    return Err("an input is listed twice");
                }
                total_in = total_in.checked_add(i.value).ok_or("overflow")?;
                ins.push(utxo::Input { txid, vout: i.vout, value: i.value });
            }
            let mut outs = vec![];
            let (mut total_out, mut change): (u64, u64) = (0, 0);
            for o in outputs {
                let script = hex::decode(&o.script).map_err(|_| "scripts must be hex")?;
                if o.value == 0 {
                    return Err("an output pays nothing");
                }
                if script == own {
                    change = change.checked_add(o.value).ok_or("overflow")?;
                } else {
                    if !utxo::recipient_script_ok(chain, &script) {
                        return Err("an output script isn't a standard script");
                    }
                    if !queued && !u.recipients.contains(&o.script.to_lowercase()) {
                        return Err("recipient not allowed");
                    }
                }
                total_out = total_out.checked_add(o.value).ok_or("overflow")?;
                outs.push(utxo::Output { script, value: o.value });
            }
            if total_out > total_in {
                return Err("outputs exceed inputs");
            }
            // What leaves the account: payments plus the fee. The digest
            // commits to every input's amount, so these totals are real.
            let spends = vec![(format!("utxo:{chain}"), (total_in - change) as u128)];
            let hashes = utxo::sighashes(chain, &own, &ins, &outs, *branch_id, *expiry_height);
            Plan { kind: Kind::Utxo { path: chain.clone(), hashes }, spends }
        }
        Action::HlOrder { .. } | Action::HlCancel { .. } | Action::HlLeverage { .. } => {
            if queued {
                return Err("Hyperliquid actions can't be queued");
            }
            let (action_bytes, spends) = hl_check(g, action, now)?;
            let nonce = match action {
                Action::HlOrder { nonce, .. } | Action::HlCancel { nonce, .. } | Action::HlLeverage { nonce, .. } => *nonce,
                _ => unreachable!(),
            };
            Plan { kind: Kind::Evm { path: EVM_PATH.to_string(), hash: hl::signing_hash(&action_bytes, nonce) }, spends }
        }
    };
    for (asset, amount) in &plan.spends {
        let cap = cap_for(g, asset).ok_or("no cap for this asset")?;
        let total = s.spent_on(asset, now).checked_add(*amount).ok_or("overflow")?;
        if total > cap {
            return Err("over the daily cap");
        }
    }
    Ok(plan)
}

/// A Hyperliquid action's encoding and spend, if the grant allows it.
pub fn hl_check(g: &Grant, action: &Action, now: u64) -> Result<(Vec<u8>, Vec<(String, u128)>), &'static str> {
    let (asset, nonce) = match action {
        Action::HlOrder { asset, nonce, .. } | Action::HlCancel { asset, nonce, .. } | Action::HlLeverage { asset, nonce, .. } => (*asset, *nonce),
        _ => return Err("not a Hyperliquid action"),
    };
    if nonce + HL_NONCE_WINDOW_MS < now || nonce > now + HL_NONCE_WINDOW_MS {
        return Err("Hyperliquid nonce must be within 5 minutes of now");
    }
    let market = g.hl_markets.iter().find(|m| m.asset == asset).ok_or("Hyperliquid market not allowed")?;
    Ok(match action {
        Action::HlOrder { is_buy, price, size, reduce_only, tif, .. } => {
            if !hl::TIFS.contains(&tif.as_str()) {
                return Err("order type must be Gtc, Ioc or Alo");
            }
            let value = hl::notional_micro_usd(price, size)?;
            (hl::order_action(asset, *is_buy, price, size, *reduce_only, tif), vec![(HL_CAP_ASSET.to_string(), value)])
        }
        Action::HlCancel { oid, .. } => (hl::cancel_action(asset, *oid), vec![]),
        Action::HlLeverage { is_cross, leverage, .. } => {
            if *leverage == 0 || *leverage > market.max_leverage {
                return Err("leverage above what the grant allows");
            }
            (hl::leverage_action(asset, *is_cross, *leverage), vec![])
        }
        _ => unreachable!(),
    })
}

#[derive(near_sdk::serde::Deserialize)]
#[serde(crate = "near_sdk::serde")]
struct FtArgs {
    receiver_id: AccountId,
    amount: U128,
}
