//! The rules a session key works within. Pure functions, so every rule can
//! be tested on its own.

use crate::evm::{self, Decoded, Eip1559};
use crate::hl;
use crate::utxo;
use crate::sol;
use crate::xrpl;
use crate::stellar;
use crate::tron;
use crate::ton;
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

/// A Solana venue: a program with a decoder in `sol.rs`, and the pool or
/// market (`scope`) the agent may use there, base58.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SolVenue {
    pub program: String,
    pub scope: String,
}

/// What the agent may do on Solana. Caps use `sol` (lamports, wrapped SOL
/// included, fees and rent counted), `spl:<mint>`, for Kamino borrows
/// `borrow:solana:<mint>`, and for Tensor listings `nft:solana` (a count).
/// A Tensor venue's scope is the marketplace program itself.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SolGrant {
    /// The account's Solana address (Chain Signatures, path `solana`), as
    /// the passkey saw it.
    pub self_address: String,
    pub venues: Vec<SolVenue>,
    /// Addresses SOL and token payments may go to.
    pub recipients: Vec<String>,
    /// Tensor: the lowest price (lamports) the agent may list an NFT at,
    /// so it can't sell one cheaply to an accomplice. Zero: no listing.
    #[serde(default)]
    pub nft_floor: U128,
}

/// What the agent may do on the XRP Ledger. Caps use `xrp` (drops, fees
/// counted) and `xrpl:<currency>.<issuer>` (millionths of a unit).
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XrplGrant {
    /// The account's classic address and its key (Chain Signatures, path
    /// `xrp`, compressed, hex), as the passkey saw them.
    pub self_address: String,
    pub public_key: String,
    /// Issued currencies it may hold and trade, as `<currency>.<issuer>`.
    /// XRP is always allowed.
    pub assets: Vec<String>,
    pub recipients: Vec<String>,
    /// The ledger's order book.
    pub dex: bool,
    /// AMM deposits and withdrawals.
    pub amm: bool,
    /// Minting and trading NFTs.
    pub nft: bool,
}

/// What the agent may do on Stellar. Caps use `xlm` and
/// `stellar:<code>:<issuer>`, in stroops (fees counted in `xlm`).
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StellarGrant {
    /// The account's G… address (Chain Signatures, path `stellar`).
    pub self_address: String,
    /// Issued assets it may hold and trade, as `<code>:<issuer>`. XLM is
    /// always allowed.
    pub assets: Vec<String>,
    pub recipients: Vec<String>,
    pub dex: bool,
    pub amm: bool,
    /// Blend pools (C… contracts) it may supply to, borrow from, repay and
    /// withdraw from. Caps: `xlm` or `soroban:<token>` for what the pool
    /// pulls, `borrow:stellar:<token>` for borrows.
    #[serde(default)]
    pub blend: Vec<String>,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TronVenue {
    /// T… contract address.
    pub contract: String,
    /// 0x-prefixed 4-byte selector.
    pub selector: String,
}

/// What the agent may do on TRON. Caps use `trx` (sun; call values and
/// energy fee limits counted), `trc20:<contract>` and `borrow:tron:<jToken>`.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TronGrant {
    /// The account's T… address (Chain Signatures, path `tron`).
    pub self_address: String,
    pub venues: Vec<TronVenue>,
    pub recipients: Vec<String>,
    /// TRC-721 contracts whose NFTs it may move or approve a marketplace for.
    #[serde(default)]
    pub nfts: Vec<String>,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TonJetton {
    /// The jetton's master contract.
    pub master: String,
    /// The account's own jetton wallet for it (the master's
    /// get_wallet_address for the account), approved with the passkey.
    pub wallet: String,
}

/// What the agent may do on TON. Caps use `ton` (nanotons; gas attached to
/// calls counted), `jetton:<master>` and `borrow:ton:<EVAA asset id>`.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TonGrant {
    /// The account's wallet address (v4r2, the MPC key on path `ton`).
    pub self_address: String,
    pub recipients: Vec<String>,
    #[serde(default)]
    pub jettons: Vec<TonJetton>,
    /// EVAA lending masters it may supply to and withdraw (borrow) from.
    #[serde(default)]
    pub evaa: Vec<String>,
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
    /// Seaport listings: the lowest price (wei, to the account) the agent
    /// may list an NFT at. Zero: no listing.
    #[serde(default)]
    pub evm_nft_floor: U128,
    /// Payment-only chains the agent may pay on.
    #[serde(default)]
    pub utxo: Vec<UtxoGrant>,
    /// Solana, if the agent may act there.
    #[serde(default)]
    pub sol: Option<SolGrant>,
    #[serde(default)]
    pub xrpl: Option<XrplGrant>,
    #[serde(default)]
    pub stellar: Option<StellarGrant>,
    #[serde(default)]
    pub tron: Option<TronGrant>,
    #[serde(default)]
    pub ton: Option<TonGrant>,
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
    /// A Solana transaction: its instructions, a recent blockhash, and the
    /// bumps of the associated token accounts it uses.
    /// An XRP Ledger transaction from typed fields.
    XrplTx { tx: xrpl::XrplTx },
    /// A Stellar transaction from typed operations.
    StellarTx { tx: stellar::StellarTx },
    /// A TRON transaction from typed fields.
    TronTx { tx: tron::TronTx },
    /// A TON wallet message from typed messages.
    TonTx { tx: ton::TonTx },
    /// A Seaport listing for the account's EVM key to sign (EIP-712).
    SeaportOrder { order: crate::seaport::SeaportOrder },
    /// A NEAR Intents top-up: 1Click's signed quote (the exact JSON it
    /// signed, and its signature) and the transfer to the quote's deposit
    /// address (intents.rs).
    IntentsDeposit { quote: String, signature: String, action: Box<Action> },
    SolTx {
        recent_blockhash: String,
        instructions: Vec<SolIx>,
        #[serde(default)]
        atas: Vec<SolAta>,
        /// The account's Kamino obligations the transaction uses.
        #[serde(default)]
        obligations: Vec<SolObligation>,
    },
}

#[near(serializers = [json])]
#[derive(Clone, Debug)]
pub struct SolIx {
    pub program: String,
    pub accounts: Vec<SolMeta>,
    /// Instruction data, hex.
    pub data: String,
}

#[near(serializers = [json])]
#[derive(Clone, Debug)]
pub struct SolMeta {
    pub pubkey: String,
    #[serde(default)]
    pub signer: bool,
    #[serde(default)]
    pub writable: bool,
}

/// An associated token account the transaction uses: its owner, mint, token
/// program and bump. The extension derives the address itself.
#[near(serializers = [json])]
#[derive(Clone, Debug)]
pub struct SolAta {
    pub owner: String,
    pub mint: String,
    pub token_program: String,
    pub bump: u8,
}

/// The account's own Kamino obligation in a market: the default one (tag
/// 0, id 0, no seed accounts, as klend-sdk's VanillaObligation), at the
/// address the extension derives from the account, the market and `bump`.
/// Like an ATA's, a wrong bump gives an address no obligation lives at.
#[near(serializers = [json])]
#[derive(Clone, Debug)]
pub struct SolObligation {
    pub market: String,
    pub bump: u8,
}

#[near(serializers = [json])]
#[derive(Clone, Debug)]
pub struct UtxoIn {
    /// As explorers show it, hex.
    pub txid: String,
    pub vout: u32,
    pub value: u64,
    /// Dogecoin only: the whole previous transaction, hex.
    #[serde(default)]
    pub prev_tx: Option<String>,
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
    /// An ed25519 signature over the message, on the chain's own path.
    Eddsa { path: String, message: Vec<u8> },
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
            return Err("agents can pay only on Bitcoin, Litecoin, Bitcoin Cash, Zcash and Dogecoin");
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
    let mut sol_items = 0;
    if let Some(sg) = &g.sol {
        sol::b58(&sg.self_address)?;
        for v in &sg.venues {
            if !(sol::STAKE_POOL_PROGRAMS.contains(&v.program.as_str()) || v.program == sol::KAMINO || v.program == sol::TENSOR) {
                return Err("no decoder for that Solana program");
            }
            sol::b58(&v.scope)?;
        }
        for r in &sg.recipients {
            sol::b58(r)?;
        }
        sol_items = sg.venues.len() + sg.recipients.len();
    }
    let mut xrpl_items = 0;
    if let Some(x) = &g.xrpl {
        let id = xrpl::account_id(&x.self_address)?;
        let pk = hex::decode(&x.public_key).map_err(|_| "XRPL key must be hex")?;
        if pk.len() != 33 || !(pk[0] == 2 || pk[0] == 3) || xrpl::key_account(&pk) != id {
            return Err("the XRPL key isn't the account's");
        }
        for a in &x.assets {
            let (cur, iss) = a.split_once('.').ok_or("XRPL assets are <currency>.<issuer>")?;
            xrpl::account_id(iss)?;
            if cur.is_empty() || cur == "XRP" {
                return Err("bad XRPL currency");
            }
        }
        for r in &x.recipients {
            xrpl::account_id(r)?;
        }
        xrpl_items = x.assets.len() + x.recipients.len();
    }
    let mut stellar_items = 0;
    if let Some(x) = &g.stellar {
        stellar::account(&x.self_address)?;
        for a in &x.assets {
            let (code, iss) = a.split_once(':').ok_or("Stellar assets are <code>:<issuer>")?;
            stellar::account(iss)?;
            if code.is_empty() || code.len() > 12 || code == "XLM" {
                return Err("bad Stellar asset code");
            }
        }
        for r in &x.recipients {
            stellar::account(r)?;
        }
        for p in &x.blend {
            stellar::contract(p)?;
        }
        stellar_items = x.assets.len() + x.recipients.len() + x.blend.len();
    }
    let mut tron_items = 0;
    if let Some(t) = &g.tron {
        tron::address(&t.self_address)?;
        for v in &t.venues {
            tron::address(&v.contract)?;
            let sel = evm::parse_hex(&v.selector)?;
            if sel.len() != 4 {
                return Err("selector must be 4 bytes");
            }
            if sel == evm::SEL_TRANSFER || sel == evm::SEL_APPROVE {
                return Err("transfer and approve can't be venues; use caps and recipients");
            }
        }
        for r in t.recipients.iter().chain(&t.nfts) {
            tron::address(r)?;
        }
        tron_items = t.venues.len() + t.recipients.len() + t.nfts.len();
    }
    let mut ton_items = 0;
    if let Some(t) = &g.ton {
        ton::address(&t.self_address)?;
        for r in t.recipients.iter().chain(&t.evaa) {
            ton::address(r)?;
        }
        for j in &t.jettons {
            ton::address(&j.master)?;
            ton::address(&j.wallet)?;
        }
        ton_items = t.recipients.len() + t.evaa.len() + t.jettons.len();
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
        // Solana mints are base58, which is case-sensitive.
        if !c.asset.starts_with("spl:") && !c.asset.starts_with("xrpl:") && !c.asset.starts_with("stellar:") && !c.asset.starts_with("trc20:") && !c.asset.starts_with("borrow:tron:") && !c.asset.starts_with("jetton:") && !c.asset.starts_with("soroban:") && !c.asset.starts_with("borrow:stellar:") && !c.asset.starts_with("borrow:solana:") && c.asset != c.asset.to_lowercase() {
            return Err("cap assets must be lowercase");
        }
    }
    if g.near_venues.len() + g.evm_venues.len() + g.near_recipients.len() + g.evm_recipients.len() + g.caps.len() + g.hl_markets.len() + g.utxo.len() + sol_items + xrpl_items + stellar_items + tron_items + ton_items > 128 {
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
                if method == "ft_transfer_call" {
                    ft_call_msg_ok(g, &a.receiver_id, &a.msg, queued)?;
                } else if !a.msg.is_empty() {
                    return Err("ft_transfer takes no msg");
                }
                spends.push((format!("nep141:{receiver_id}"), a.amount.0));
            } else {
                if !queued && !g.near_venues.iter().any(|v| &v.receiver_id == receiver_id && &v.method == method) {
                    return Err("not an allowed venue");
                }
                spends.extend(rhea_call(receiver_id, method, args)?);
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
                    // setApprovalForAll hands every NFT of the collection to
                    // the operator: only to a venue (Seaport), queued or not.
                    if selector == evm::SEL_SET_APPROVAL_FOR_ALL {
                        if t.value > 0 || t.data.len() != 4 + 64 || word_u128_ok(&t.data[36..68]) > 1 {
                            return Err("bad setApprovalForAll");
                        }
                        let op = evm::fmt_address(&evm::word_address(&t.data[4..36])?);
                        if !g.evm_venues.iter().any(|v| v.chain_id == chain && v.to.to_lowercase() == op) {
                            return Err("operator is not a venue");
                        }
                    }
                    // Where a venue call's result goes, for layouts we know:
                    // always the account itself, queued or not.
                    if let Some(b) = evm::beneficiary(&t.data)? {
                        let own = g.evm_self.as_deref().ok_or("this grant doesn't name the account's EVM address")?;
                        if b != evm::parse_address(own)? {
                            return Err("venue call sends the result to someone else");
                        }
                    }
                    // Borrowing counts against its own cap: `borrow:evm:<chain>:<token>`.
                    if let Some((asset, amount)) = evm::aave_borrow(&t.data)? {
                        spends.push((format!("borrow:evm:{chain}:{}", evm::fmt_address(&asset)), amount));
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
                if chain == "dogecoin" {
                    // The digest doesn't commit to amounts: read the real one.
                    let prev = hex::decode(i.prev_tx.as_deref().ok_or("Dogecoin inputs need their previous transaction")?).map_err(|_| "previous transaction must be hex")?;
                    let (value, script) = utxo::prev_output(&prev, &txid, i.vout)?;
                    if value != i.value || script != own {
                        return Err("input isn't the account's coin of that amount");
                    }
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
        Action::XrplTx { tx } => {
            let x = g.xrpl.as_ref().ok_or("XRP Ledger not allowed")?;
            let (_, hash, spends) = xrpl::check(x, tx, queued)?;
            Plan { kind: Kind::Evm { path: "xrp".into(), hash }, spends }
        }
        Action::TronTx { tx } => {
            let t = g.tron.as_ref().ok_or("TRON not allowed")?;
            let (_, id, spends) = tron::check(t, tx, queued)?;
            Plan { kind: Kind::Evm { path: "tron".into(), hash: id }, spends }
        }
        Action::SeaportOrder { order } => {
            let (hash, spends) = crate::seaport::check(g, order, queued)?;
            Plan { kind: Kind::Evm { path: EVM_PATH.to_string(), hash }, spends }
        }
        Action::TonTx { tx } => {
            let t = g.ton.as_ref().ok_or("TON not allowed")?;
            let (_, hash, spends) = ton::check(t, tx, queued)?;
            Plan { kind: Kind::Eddsa { path: "ton".into(), message: hash.to_vec() }, spends }
        }
        Action::StellarTx { tx } => {
            let x = g.stellar.as_ref().ok_or("Stellar not allowed")?;
            let (_, hash, spends) = stellar::check(x, tx, queued, stellar::TESTNET)?;
            Plan { kind: Kind::Eddsa { path: "stellar".into(), message: hash.to_vec() }, spends }
        }
        Action::SolTx { recent_blockhash, instructions, atas, obligations } => {
            let sg = g.sol.as_ref().ok_or("Solana not allowed")?;
            let (message, spends) = sol_message(sg, recent_blockhash, instructions, atas, obligations, queued)?;
            Plan { kind: Kind::Eddsa { path: "solana".into(), message }, spends }
        }
        Action::IntentsDeposit { quote, signature, action: inner } => {
            if queued {
                return Err("a top-up can't be queued");
            }
            if matches!(**inner, Action::IntentsDeposit { .. }) {
                return Err("one top-up at a time");
            }
            let d = crate::intents::check(g, quote, signature, now, wallet)?;
            // The deposit address may be paid in this transfer only.
            let mut s2 = s.clone();
            match d.origin {
                crate::intents::Family::Evm(c) => s2.grant.evm_recipients.push(format!("{c}:{}", d.address)),
                crate::intents::Family::Sol => s2.grant.sol.as_mut().ok_or("Solana not allowed")?.recipients.push(d.address.clone()),
                _ => return Err("the extension can't pay a deposit address on that chain yet"),
            }
            let p = check(&s2, inner, Mode::Direct, now, wallet, extension, signer)?;
            if !p.spends.iter().any(|(a, _)| *a == d.spend) {
                return Err("the transfer doesn't move the quoted asset");
            }
            for (a, v) in &p.spends {
                let max = if *a == d.spend {
                    // A SOL deposit also pays the transaction's fee.
                    if a == "sol" { d.amount_in + crate::intents::SOL_FEE_ALLOWANCE } else { d.amount_in }
                } else if a == "sol" && d.origin == crate::intents::Family::Sol {
                    crate::intents::SOL_FEE_ALLOWANCE
                } else {
                    return Err("a top-up moves only the quoted asset");
                };
                if *v > max {
                    return Err("more than the quoted amount");
                }
            }
            p
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
    #[serde(default)]
    msg: String,
}

/// Ref Finance's exchange contracts (mainnet, testnet): `ref-finance-101.testnet`'s
/// metadata() reports version 1.9.19.
pub const REF_EXCHANGES: [&str; 2] = ["v2.ref-finance.near", "ref-finance-101.testnet"];

/// Rhea Lending (formerly Burrow): mainnet, and the testnet contract
/// rhea-finance/burrow-sdk-python names (its get_config enables the Pyth
/// oracle). Actions as that SDK builds them.
pub const RHEA_LENDING: [&str; 2] = ["contract.main.burrow.near", "contract.1689937928.burrow.testnet"];
/// The price oracles whose `oracle_call` forwards to it.
pub const RHEA_ORACLES: [&str; 2] = ["priceoracle.near", "mock-priceoracle.testnet"];

/// Checks a list of Rhea Lending actions: each one of `allowed`, carrying
/// only a token and amounts (no account field, so nothing goes anywhere
/// but the caller). Returns what is borrowed, as `borrow:near:<token>`.
fn rhea_actions(v: &near_sdk::serde_json::Value, allowed: &[&str]) -> Result<Vec<(String, u128)>, &'static str> {
    let actions = v.as_array().ok_or("bad lending message")?;
    if actions.is_empty() || actions.len() > 4 {
        return Err("1 to 4 lending actions");
    }
    let mut borrows = vec![];
    for a in actions {
        let a = a.as_object().filter(|a| a.len() == 1).ok_or("bad lending message")?;
        let (kind, body) = a.iter().next().unwrap();
        if !allowed.contains(&kind.as_str()) {
            return Err("a lending action that isn't allowed here");
        }
        let body = body.as_object().ok_or("bad lending message")?;
        if body.keys().any(|k| k != "token_id" && k != "amount" && k != "max_amount") {
            return Err("lending action fields other than token and amounts");
        }
        let token = body.get("token_id").and_then(|t| t.as_str()).ok_or("bad lending message")?;
        let amount = |k: &str| -> Result<Option<u128>, &'static str> {
            body.get(k).map(|x| x.as_str().and_then(|s| s.parse().ok()).ok_or("bad lending amount")).transpose()
        };
        let amt = amount("amount")?;
        amount("max_amount")?;
        if kind == "Borrow" {
            borrows.push((format!("borrow:near:{token}"), amt.ok_or("a borrow needs an amount")?));
        }
    }
    Ok(borrows)
}

/// Checks a call to Rhea Lending (`execute`, `execute_with_pyth`) or to its
/// oracle (`oracle_call`); returns the borrows. Other calls: no spends.
fn rhea_call(receiver_id: &AccountId, method: &str, args: &str) -> Result<Vec<(String, u128)>, &'static str> {
    const OUT: [&str; 4] = ["Withdraw", "Borrow", "IncreaseCollateral", "DecreaseCollateral"];
    let v: near_sdk::serde_json::Value = near_sdk::serde_json::from_str(args).map_err(|_| "bad lending message")?;
    let o = v.as_object().ok_or("bad lending message")?;
    if RHEA_LENDING.contains(&receiver_id.as_str()) && (method == "execute" || method == "execute_with_pyth") {
        if o.len() != 1 {
            return Err("bad lending message");
        }
        return rhea_actions(o.get("actions").ok_or("bad lending message")?, &OUT);
    }
    if RHEA_ORACLES.contains(&receiver_id.as_str()) && method == "oracle_call" {
        if o.keys().any(|k| k != "receiver_id" && k != "msg") {
            return Err("bad lending message");
        }
        let to = o.get("receiver_id").and_then(|r| r.as_str()).ok_or("bad lending message")?;
        if !RHEA_LENDING.contains(&to) {
            return Err("the oracle may only call Rhea Lending");
        }
        let m: near_sdk::serde_json::Value = near_sdk::serde_json::from_str(o.get("msg").and_then(|m| m.as_str()).ok_or("bad lending message")?).map_err(|_| "bad lending message")?;
        let m = m.as_object().filter(|m| m.len() == 1).ok_or("bad lending message")?;
        return rhea_actions(m.get("actions").ok_or("bad lending message")?, &OUT);
    }
    Ok(vec![])
}

/// What an `ft_transfer_call` may carry to a contract. An empty message
/// (a plain deposit, credited to the account) to any venue. To Ref, a swap
/// whose every field is one seen in Ref's own swaps, through pools the grant
/// lists (venue method `pool:<id>`): Ref pools are permissionless, so an
/// unlisted one could be the agent's own at a ruinous price, and an
/// unknown field could send the output elsewhere. The output comes back to
/// the account, the sender. Any other message is refused.
fn ft_call_msg_ok(g: &Grant, to: &AccountId, msg: &str, queued: bool) -> Result<(), &'static str> {
    if msg.is_empty() {
        return Ok(());
    }
    if RHEA_LENDING.contains(&to.as_str()) {
        // Tokens in: as collateral, or to repay (burrow-sdk-python
        // deposit_collateral, repay_from_wallet).
        let v: near_sdk::serde_json::Value = near_sdk::serde_json::from_str(msg).map_err(|_| "bad lending message")?;
        let o = v.as_object().filter(|o| o.len() == 1).and_then(|o| o.get("Execute")).and_then(|e| e.as_object()).ok_or("bad lending message")?;
        if o.len() != 1 {
            return Err("bad lending message");
        }
        let spends = rhea_actions(o.get("actions").ok_or("bad lending message")?, &["IncreaseCollateral", "Repay"])?;
        debug_assert!(spends.is_empty());
        return Ok(());
    }
    if !REF_EXCHANGES.contains(&to.as_str()) {
        return Err("only a plain deposit (empty msg) to this contract");
    }
    let v: near_sdk::serde_json::Value = near_sdk::serde_json::from_str(msg).map_err(|_| "bad Ref message")?;
    let o = v.as_object().ok_or("bad Ref message")?;
    // `force: 0` as Rhea's own SDK (rhea-finance/rhea-sdk, instantSwap) sends
    // it; `skip_unwrap_near` as rnear-sdk's instant unstake does (whether
    // wNEAR output comes back as NEAR; to the sender either way).
    if o.keys().any(|k| k != "actions" && k != "force" && k != "skip_unwrap_near")
        || o.get("force").is_some_and(|f| f.as_u64() != Some(0))
        || o.get("skip_unwrap_near").is_some_and(|f| !f.is_boolean())
    {
        return Err("Ref message fields other than actions");
    }
    let actions = o.get("actions").and_then(|a| a.as_array()).ok_or("bad Ref message")?;
    if actions.is_empty() || actions.len() > 4 {
        return Err("1 to 4 Ref swaps");
    }
    for (i, a) in actions.iter().enumerate() {
        let a = a.as_object().ok_or("bad Ref message")?;
        const FIELDS: [&str; 5] = ["pool_id", "token_in", "token_out", "amount_in", "min_amount_out"];
        if a.keys().any(|k| !FIELDS.contains(&k.as_str())) {
            return Err("Ref swap fields other than pool, tokens and amounts");
        }
        let pool = a.get("pool_id").and_then(|p| p.as_u64()).ok_or("bad Ref message")?;
        // A route's middle hops carry 0; its last hop holds the minimum.
        let min = a.get("min_amount_out").and_then(|m| m.as_str()).ok_or("bad Ref message")?;
        if min.is_empty() || !min.bytes().all(|b| b.is_ascii_digit()) {
            return Err("bad Ref message");
        }
        if i == actions.len() - 1 && min.bytes().all(|b| b == b'0') {
            return Err("a Ref swap needs a minimum output");
        }
        if !queued && !g.near_venues.iter().any(|v| &v.receiver_id == to && v.method == format!("pool:{pool}")) {
            return Err("not an allowed Ref pool");
        }
    }
    Ok(())
}

/// Checks a Solana transaction against the grant and compiles its message.
pub fn sol_message(sg: &SolGrant, blockhash: &str, ixs: &[SolIx], atas: &[SolAta], obligations: &[SolObligation], queued: bool) -> Result<(Vec<u8>, Vec<(String, u128)>), &'static str> {
    let me = sol::b58(&sg.self_address)?;
    if atas.len() > 12 || obligations.len() > 4 {
        return Err("too many token accounts");
    }
    let mut own = sol::Own { me, atas: vec![], obligations: vec![] };
    for o in obligations {
        own.obligations.push(sol::kamino_obligation(&me, &sol::b58(&o.market)?, o.bump));
    }
    for a in atas {
        let (o, m, tp) = (sol::b58(&a.owner)?, sol::b58(&a.mint)?, sol::b58(&a.token_program)?);
        if a.token_program != sol::TOKEN && a.token_program != sol::TOKEN_2022 {
            return Err("unknown token program");
        }
        own.atas.push((sol::ata(&o, &m, &tp, a.bump), o, m));
    }
    let ixs = sol::parse(ixs)?;
    sol::check(sg, &own, &ixs, &sol::b58(blockhash)?, queued)
}

/// A word's value, or u128::MAX if it doesn't fit.
fn word_u128_ok(w: &[u8]) -> u128 {
    evm::word_u128(w).unwrap_or(u128::MAX)
}
