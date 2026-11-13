# Marvii on NEAR

Marvii is a passkey wallet: self-custody secured by your phone's Face ID or fingerprint, with no seed phrase. It has one account on every chain, and agents and apps act only within limits you set.

- **Your passkey owns the account.** It's a [`defuse-wallet`](https://github.com/near/intents) contract that checks the passkey's P-256 signature on-chain (NEAR's `p256_verify`). No server holds a key that can move your funds.
- **One account, many chains.** The same account holds NEAR, has one address on every EVM chain, and has its own address on Bitcoin, Solana, TRON, XRP, Cardano, Litecoin, Dogecoin, Bitcoin Cash and Zcash, all signed through NEAR Chain Signatures.
- **Agents and apps get session keys** with rules NEAR checks on every request:
  - which protocols they may use, on every chain the account has. The extension knows each protocol's calls and checks that whatever comes back goes only to the account itself:
    - EVM: Aave V3, Uniswap V3, Lido, Seaport (OpenSea) listings, Polymarket orders, and any contract and function the owner lists
    - NEAR: Ref Finance, Rhea Lending, Stader NearX, and other contracts the owner lists (the demo agent also stakes on LiNEAR, Meta Pool and rNEAR this way)
    - Solana: Jupiter, Phoenix (spot and perps), Jito, Kamino, Tensor
    - TRON: JustLend (including sTRX staking), SunSwap; TON: Tonstakers, EVAA; XRP Ledger and Stellar: their order books and AMMs; Stellar: Blend
    - Hyperliquid: which markets, and at what leverage
  - on payment-only chains (Bitcoin, Litecoin, Dogecoin, Bitcoin Cash, Zcash, Cardano), who they may pay, with change only back to the account and the fee counted against the cap
  - who they may pay
  - a daily cap per asset
  - an expiry, and an inactivity expiry
- **Outside the rules, you decide:**
  - Over the cap, the request waits for one passkey tap.
  - New recipients and contracts wait in a delay queue, where the passkey can cancel them.
  - Revoke any agent instantly.
- **Agents' keys live in a TEE.** The page approves only agent keys made inside Marvii's module on OutLayer, NEAR's TEE compute (Intel TDX), and checks the OutLayer run that made each one on NEAR. A key is derived inside the enclave for one exact build of our code and never leaves it, so not even the app's operator can read it; each run's TDX quote names that build.
- **Apps ask with a link.** An agent built with `@replico-labs/marvii-sdk` sends the owner a permission link; the page shows exactly what it asks for, in plain words, before the passkey approves.
- **Swap quotes and top-ups through NEAR Intents.** Live prices between every chain's coin, and top-ups that move your own funds to your own address on another chain. An agent can top up too: the extension checks 1Click's signed quote and that the swap pays and refunds only you.

Testnet only (top-ups through NEAR Intents use real mainnet funds). The contracts are unaudited.

## What happened on testnet

These ran on NEAR testnet and each chain's testnet (8 to 13 November 2026), or, for protocols that exist only on mainnet (Jupiter, Phoenix, Polymarket), on a local fork of mainnet: Aave (supply, borrow, repay, withdraw), Uniswap and Seaport on Base Sepolia; Lido on Hoodi; Hyperliquid testnet; Jito, Kamino and Tensor on Solana devnet; Ref, Rhea Lending, NearX, rNEAR, Meta Pool and LiNEAR on NEAR; SunSwap, JustLend, sTRX and NFTs on TRON (Nile); EVAA and tsTON on TON; the order book, AMM and NFTs on the XRP Ledger; the order book, AMM and Blend on Stellar; Bitcoin, Zcash and Cardano payments; Phoenix perps from an agent whose key is in a TEE; and the refusals for each. A summary table, transaction hashes and explorer links are in [TESTNET.md](TESTNET.md).

## How it works

```
 passkey (phone) ──signs──▶ wallet contract (defuse-wallet, P-256)
                                 │  w_execute_signed: anything the owner wants
                                 │
 agent key ──signs──▶ Marvii extension ──checks rules──▶ w_execute_extension
                                                        │
                       NEAR call to a listed venue  ◀───┤
                       or Chain Signatures `sign`  ◀────┘ (one key per chain: "evm", "solana", "ton", …)
                                 │
                       agent checks the signature and broadcasts on that chain
                       (or sends the Hyperliquid action with it)
```

- **`extension/`** is the session-key extension (Rust, near-sdk). The wallet's owner installs it, and it holds no funds.
  - It decodes every request itself and builds what gets signed: NEAR calls and NEP-141 transfers, EIP-1559 transactions, Solana messages, TRON, TON, XRP Ledger and Stellar transactions, Bitcoin-style and Cardano payments, and the off-chain orders of Hyperliquid, Polymarket and Seaport. It never signs bytes it is handed.
  - For venue calls whose layout it knows (Aave `supply`, `withdraw`, `repay`; Uniswap V3 `exactInputSingle`), it reads where the result goes (`onBehalfOf`, `to`, `recipient`). That must be the account's own EVM address, named in the grant the passkey signed, so an agent can't deposit or swap for someone else. Extra or dirty data is refused (extension v5).
  - For Hyperliquid, the agent sends an order's fields, never bytes to sign. The extension encodes the action as Hyperliquid's SDK does (MessagePack) and builds the EIP-712 digest itself, with the source fixed to Hyperliquid testnet. A signature from it is valid only on that exchange and network, and can't be an EVM transaction.
  - Order value (price × size) counts against a daily USD cap; leverage is capped per market; the nonce must be within 5 minutes of now.
  - On Solana, each instruction is checked against the program's layout: the account's own token accounts in and out, no extra fee, slippage limits. Phoenix perps runs the way protean-bot's desk does: the account's key may only move collateral between its own Phoenix accounts and delegate an isolated subaccount to the account's own trading key, which may only place limit orders on listed markets, up to a set size, and can never withdraw.
  - For Bitcoin, Litecoin, Bitcoin Cash and Zcash, the agent sends a payment's inputs and outputs (at most 2 and 3), and the extension builds every signing hash itself: BIP-143, BIP-143 with FORKID, and ZIP-244. Each of these digests commits to the amounts being spent, so an agent that misstates them gets a signature the chain rejects. The totals the extension counts are real: everything that leaves the account, fee included, counts against the cap. Every output must go to a listed recipient or back to the account. Dogecoin's legacy digest doesn't commit to amounts, so for Dogecoin the agent also sends each input's previous transaction and the extension reads the amounts from it. Cardano payments are built the same way (a Shelley body; the fee is in it). The extension's hashes are tested to equal core's, which match bitcoinjs-lib, bitcore-lib-cash and zcash_primitives.
  - It checks venue, recipient and cap.
  - It has the wallet act through `w_execute_extension`. The wallet itself calls the MPC signer, so the EVM address belongs to the wallet, not to the extension.
  - A session can never call the wallet, the extension or the signer directly, and never sends wallet operations.
- **`core/`** is the wallet's own Rust types compiled to WebAssembly, so the page never assembles signed bytes by hand. It builds:
  - the account ID
  - the exact hash the passkey signs
  - the WebAuthn proof
  - the EVM transaction decoding and hash, sharing the extension's `evm.rs` source
- **`relayer/`** pays gas and nothing else. It only relays for wallets built from our code, and checks the passkey's proof before spending gas. Everything it forwards is checked again on-chain.
- **`web/`** is the page:
  - create with a passkey
  - Home: the account, then Assets, Swap, and Apps & agents
  - Assets: a tile per chain. Each chain's page has Send and Receive (address and QR code), what it holds, and its recent history from the chain's explorer. Send works on every chain: NEAR, nine EVM chains, Bitcoin, Solana, TRON, XRP, Cardano, Litecoin, Dogecoin, Bitcoin Cash and Zcash
  - Swap: a menu with two pages. Swap: a live NEAR Intents quote (1Click, `dry: true`). Top up another chain: a real quote paying your own address, deposited with the passkey (mainnet funds)
  - Hyperliquid (on the HyperEVM page): the account's spot and perps USDC, and Move to perps with the passkey
  - Apps & agents: a dropdown of protocols by chain; each protocol's page lists the agents that can use it, what they may do there and their daily limits, with Revoke. Access comes from permission links (pasted here or opened directly); a form to add a key by hand stays under "Building an agent?" for testing
  - Confirm, which decodes exactly what the passkey signs
  - approvals, the queue, and a one-tap move to a newer extension
- **`sdk/`** is `@replico-labs/marvii-sdk`: an agent's key in OutLayer's TEE, permission links, and a builder for every action type.
- **`agent/`** holds demo agents. Each holds only a session key and checks every signature against the account's address before broadcasting: `agent.mjs` (Aave, Uniswap and payments on Base Sepolia, Hyperliquid testnet), `eth.mjs` (Lido, Seaport), `near.mjs` (Ref, Rhea, NearX), `solana.mjs`, `tron.mjs`, `ton.mjs`, `xrpl.mjs`, `stellar.mjs`, `intents.mjs` (top-ups), and on mainnet forks `jupiter.mjs`, `phoenix.mjs`, `phoenix-perps.mjs` and `polymarket.mjs`.
  - **`agent/outlayer/`** holds the agent's key in OutLayer's TEE. A Rust module (wasm32-wasip2, 171 KB) declares one ed25519 key bound to its own build (`bind: "wasm"`). OutLayer derives the key inside the enclave and signs only through host calls, so the module never sees it. The module builds the agent's message itself, for the action types the extension knows (Hyperliquid, EVM, Solana and Phoenix perps orders, the other chains, Seaport, Polymarket, Intents top-ups), with the deadline taken from the NEAR block's time; it never signs bytes it is handed. Each run is a NEAR transaction to `outlayer.testnet`, and `testnet-api.outlayer.ai/attestations/by-tx/<tx>` returns its TDX quote. `agent.mjs` holds no key: it asks the module to sign, then sends to the relayer and Hyperliquid.
- **`names/`** and **`forwarder/`** give each account a name, e.g. `alice.marvii.testnet`. NEAR wallet apps and exchanges don't yet accept `0s…` accounts (NEP-616) as recipients; a name works everywhere.
  - `names/` is the registry, deployed at `marvii.testnet`. The account calls `register("alice")` itself, so only its passkey can claim a name for it, and the page puts this in the same tap that activates the account. One name per account; a name is never reassigned.
  - The registry creates `alice.marvii.testnet` with no keys, running `forwarder/` (shared global code, published once by hash), with the account as its only target. NEAR sent to the name is forwarded by the relayer within about 20 seconds (anyone may call `forward`); tokens sent with `ft_transfer_call` are forwarded at once, and with a plain `ft_transfer` by `forward_ft`.
  - The forwarder is written against near-sdk's low-level `env` without its macros, to keep the global code small (27 KB): publishing burns NEAR per byte.
- **`devtool/`** handles deployment, a demo account and end-to-end runs.

## Deployed on NEAR testnet

| | |
|---|---|
| Wallet code (`defuse-wallet-webauthn-p256`, near/intents `32cd1f9`), immutable global contract | `909f39dd861604db7f75eafad948b6cf288ffc81db1fb59177030f227721c98e` |
| Marvii extension v6 (13 November 2026) | `0e59c2a2862583fd03eb8a6ad722a67d54189ce241f4f3b46c4884e26ee4f5a0` (v3 to v5 still accepted; the page offers the move) |
| OutLayer TEE module (agent keys) | `551085926472e0a1236afa1c7461e63c87413496c63cf77645890e886433775b` on `outlayer.testnet` |
| Chain Signatures | `v1.signer-prod.testnet` |
| Names registry; forwarder code hash | `marvii.testnet`; `9428c9b6ea5401af2ecd8487170c5ceb409daa81133b8b62d70f59105d7f795e` |
| Demo account | `0s3465557e026f471c0e84898f799c8032f356c77a`, EVM `0x48f0deef2c7e15ea9b4398a6875b43b5ab6ea81e` |

## Run it

Requirements: Rust 1.97 with `wasm32-unknown-unknown`, Node 22, and a funded NEAR testnet account for the relayer.

1. **Test the contracts.** Expect 135 extension tests, 42 core tests, 6 registry tests and 8 forwarder tests to pass:
   ```sh
   cd extension && cargo test
   cd ../core && cargo test
   cd ../names && cargo test
   cd ../forwarder && cargo test
   ```
2. **Start the relayer** against the deployed contracts. Put the relayer account's key in the environment, never in a file in the repo:
   ```sh
   cd relayer
   MARVII_RELAYER_SECRET_KEY=ed25519:… MARVII_RELAYER_ACCOUNT=<account> \
   MARVII_NEAR_EXTENSION=0e59c2a2862583fd03eb8a6ad722a67d54189ce241f4f3b46c4884e26ee4f5a0 \
   MARVII_NEAR_WALLET_CODE_HASH=909f39dd861604db7f75eafad948b6cf288ffc81db1fb59177030f227721c98e \
   cargo run
   ```
   You should see `relayer on 127.0.0.1:8787`. `GET /config` returns the extension and code hash.
3. **Start the page:**
   ```sh
   cd web && npm install && npm run dev
   ```
   Then open http://localhost:5173.
   - Create with a passkey, then tap once more to activate. Expect a tile per chain under Assets.
   - Open an agent's permission link, and approve it on the Confirm screen.
4. **See any account read-only:** http://localhost:5173/?wallet=0s3465557e026f471c0e84898f799c8032f356c77a
5. **Run the browser end-to-end** (Chromium with a virtual passkey). Expect `PASS`:
   ```sh
   cd web && node e2e/testnet.mjs
   ```
   Names, both at creation and chosen later: `node e2e/names.mjs`. Moving an account from extension v3 to v4: `node e2e/move.mjs`.
6. **Trade on Hyperliquid testnet as the agent** (after the passkey grants it a market):
   ```sh
   cd agent && WALLET=0s… node agent.mjs hl-leverage 3
   node agent.mjs hl-order buy 0.01 2000
   ```
   Expect `signed by 0x…` (the account's EVM address) and Hyperliquid's answer. Orders need margin in perps: send testnet USDC to the account on Hyperliquid (it lands in spot), then use Move to perps on the page.

To host the relayer, see `relayer/Dockerfile`. Build it from this directory: `docker build -f relayer/Dockerfile .`.

## Connect your own agent

A Marvii agent never holds the account's keys: it signs requests with its own session key, and the extension on NEAR checks each one against the rules the owner granted. The SDK, `@replico-labs/marvii-sdk` in `sdk/`, does steps 1 to 4 below; its README has a quickstart.

1. **Make the agent's key inside the TEE.** `new OutLayerSigner({ moduleUrl, moduleHash, operator })` asks Marvii's OutLayer module for the key; `signer.keyProof` is the OutLayer run that made it. The live page approves only keys made there.
2. **Have the owner grant it.** `permissionLink({ publicKey, keyProof, keyId, app, grant })` gives a link to https://near.marvii.xyz. The owner opens it, sees exactly what the agent asks for (protocols, recipients, daily limits, expiry) and approves with the passkey; `waitForApproval` returns when they have. The grant attaches a small storage deposit (0.01 NEAR) from the account.
3. **Build a request.** The agent signs this exact JSON text (the field order doesn't matter to the extension, but the signature covers the exact bytes sent):
   ```json
   {
     "domain": "marvii-session-v1",
     "extension": "<extension account, from GET /config>",
     "wallet": "<the owner's account, 0s…>",
     "key_id": "my-agent",
     "nonce": 8,
     "deadline_ms": 1762700300000,
     "action": { "type": "hl_leverage", "asset": 4, "is_cross": true, "leverage": 3, "nonce": 1762700000000 }
   }
   ```
   - `nonce` is the session's current nonce plus one. Read it with the extension's view `get_session({ wallet, key_id })`.
   - `deadline_ms` is at most 10 minutes ahead of now.
   - `signature` is ed25519 over SHA-256 of the message text, as 128 hex characters.
4. **Send it** to the relayer, which pays the gas:
   ```
   POST https://relayer.marvii.xyz/agent/execute
   { "msg": "<the JSON text above>", "signature": "<hex>" }
   ```
   The answer has `ok`, `tx_hash` (the NEAR transaction), `reason` when refused, and the Chain Signatures result in `mpc_signature` (`mpc_signatures`, one per input, for a payment).
5. **Finish on the other chain.** The agent attaches the signature and broadcasts. For an EVM transaction the answer also carries `evm_signing_hash`; for Hyperliquid, the extension's view `hl_signing_hash({ action })` gives the digest. Check that the signature recovers to the account's own address before sending, as `agent.mjs` does.

| `action.type` | Fields |
|---|---|
| `evm_tx` | `tx`: `chain_id`, `nonce`, `max_priority_fee_per_gas`, `max_fee_per_gas`, `gas_limit`, `to`, `value`, `data` (0x hex). Fee and value amounts are decimal strings. |
| `near_call` | `receiver_id`, `method`, `args` (JSON text), `deposit` (yoctoNEAR, decimal string) |
| `hl_order` | `asset`, `is_buy`, `price`, `size`, `reduce_only`, `tif` (`Gtc`, `Ioc` or `Alo`), `nonce` (now, in milliseconds) |
| `hl_cancel` | `asset`, `oid`, `nonce` |
| `hl_leverage` | `asset`, `is_cross`, `leverage`, `nonce` |
| `utxo_pay` | `chain` (`bitcoin`, `litecoin`, `bitcoin-cash`, `zcash`, `dogecoin`, `cardano`), `inputs` (`txid`, `vout`, `value`), `outputs` (`script` hex, `value`); Zcash also `branch_id` and `expiry_height`. At most 2 inputs and 3 outputs. |
| `sol_tx` | `recent_blockhash`, `instructions` (`program`, `accounts`, `data` hex), and the account's own token accounts, Kamino obligations and Phoenix trader accounts it uses (`atas`, `obligations`, `traders`) |
| `phoenix_order` | `sub`, `bump`, `recent_blockhash`, `instructions`: Phoenix perps orders signed by the account's trading key for that subaccount |
| `tron_tx`, `ton_tx`, `xrpl_tx`, `stellar_tx` | `tx`: typed fields for that chain (transfers, contract calls, offers, AMM, NFTs, staking); the extension builds the transaction |
| `seaport_order`, `polymarket_order` | `order`: the order's fields; the extension builds the EIP-712 hash |
| `intents_deposit` | `quote` and `signature` from 1Click, and `action`: the transfer to the quote's deposit address |

The SDK has a builder for each (`evmTx`, `solTx`, `phoenixOrder`, …).

**Outside the rules.**
- Over the daily cap, an EVM request comes back with a `pending_id`. The owner approves it with one passkey tap on the page; the agent polls `GET /pending/<id>` for the signature.
- A new recipient or contract: send the same request to `POST /agent/propose` instead (if the grant allows proposals). It waits in a queue the owner can cancel; after the delay, `POST /queued/<id>/execute` with `{ "wallet": "<account>" }` runs it, and the rules are checked again.
- The owner can revoke the key at any time; the next request is refused.

**The TEE module.** `agent/outlayer/` is the module that holds agents' keys inside OutLayer's Intel TDX enclave. To publish a new build: `cargo build --release --target wasm32-wasip2`, then devtool's `outlayer-upload` and `outlayer-run`, and add its hash to the page's `TEE_MODULES`. The runs on testnet are in [TESTNET.md](TESTNET.md).

**Limits on the hosted relayer.** It checks the signature and nonce before paying gas, and relays at most 50 transactions per account per day. For more, run your own relayer (above).

## Security model

- The passkey is the only owner. The extension acts only within a grant the passkey signed, and it can't grant itself anything.
- Every rule has a test that attacks it with a session key and expects it to fail:
  - another venue
  - over the cap
  - an expired or revoked key
  - a replayed nonce
  - calling the wallet, the extension or the signer
  - running a queued request early or twice
  - someone else cancelling a queued request
  - the cap checked again when a queued request runs
- Agents' keys come only from Marvii's OutLayer module: the page checks, on NEAR, the OutLayer run that made each key before it lets the passkey approve it.
- The extension's owner can pause sessions. Pausing only restricts; it can't move funds or grant anything.
- Known limits:
  - testnet only, unaudited
  - the relayer's list of requests awaiting approval lives in memory. Its spending limits (per account per day, per hour, new-wallet NEAR per day and per IP) are in memory too, so a restart resets them; it checks agents' signatures and nonces before paying gas
  - passkeys belong to `near.marvii.xyz`: the live page refuses to create or use one anywhere else, so a wallet can't end up tied to a preview address. The dev server uses `localhost`; `VITE_RP_ID` overrides both
  - NEAR sent to a name waits on the name until forwarded (a plain transfer runs no code); the relayer forwards every 20 seconds
  - during the hackathon we keep a key to `marvii.testnet`. It can't touch existing names (they have no keys and their target is fixed), but it could redeploy the registry or create names directly; it's deleted before mainnet
  - caps count `approve` allowances; a venue that pulls funds another way would need its own amount rule
  - the extension never signs bare messages. Hyperliquid's EIP-712 digest is built by the extension from the order's fields, bound to Hyperliquid testnet (`source` "b", the `Exchange` domain)
  - Hyperliquid orders over the cap are refused, not offered to the passkey (only EVM transactions are)
  - Dogecoin reads and broadcasts go through a public Esplora API (QED's), which has been down since 9 November 2026; any Esplora URL can replace it (`MARVII_DOGE_ESPLORA`)
  - protocols that exist only on mainnet (Jupiter, Phoenix, Polymarket) were tested on local forks of mainnet; on the fork, Phoenix's onboarding (its API co-signs new traders) and live oracle prices were stood in for, as TESTNET.md says

## Libraries

- [`defuse-wallet`](https://github.com/near/intents) and its SDK (near/intents, MIT): the passkey wallet contract
- [near-sdk](https://github.com/near/near-sdk-rs)
- [near-kit](https://crates.io/crates/near-kit)
- NEAR Chain Signatures (`v1.signer-prod.testnet`)
- [web3icons](https://github.com/0xa3k5/web3icons) (MIT): chain and token logos, copied into `web/src/icons.ts` by `web/scripts/gen-icons.mjs`
- [qrcode-generator](https://github.com/kazuhikoarase/qrcode-generator) (MIT): Deposit QR codes
- [OutLayer](https://github.com/out-layer/outlayer): `wit/signing-keys.wit` is a copy of its signing-key interface (MIT OR Apache-2.0); [wit-bindgen](https://github.com/bytecodealliance/wit-bindgen) (Apache-2.0 WITH LLVM-exception OR MIT)
- [blake2b_simd](https://crates.io/crates/blake2b_simd) (MIT): Zcash ZIP-244 and Cardano hashes
- [webpki-roots](https://crates.io/crates/webpki-roots), tokio-rustls: the relayer's Electrum (TLS) client for Bitcoin Cash
- Reference implementations used only in tests: zcash_primitives, bitcore-lib-cash, bitcoinjs-lib, cardano-serialization-lib, hyperliquid-python-sdk
- NEAR Intents 1Click API (quotes)
- [viem](https://viem.sh): EVM transaction serialization in the demo agent
- [@ellipsis-labs/rise](https://www.npmjs.com/package/@ellipsis-labs/rise) and [@ellipsis-labs/phoenix-sdk](https://www.npmjs.com/package/@ellipsis-labs/phoenix-sdk): Phoenix perps and spot instruction layouts, in the demo agents and as the extension's reference; Jupiter's swap API (lite-api.jup.ag)
- [Surfpool](https://github.com/txtx/surfpool) and [Anvil](https://github.com/foundry-rs/foundry): local forks of Solana and Polygon mainnet
- Aave V3 Base Sepolia addresses from [bgd-labs/aave-address-book](https://github.com/bgd-labs/aave-address-book)
- [Uniswap V3](https://github.com/Uniswap/swap-router-contracts) on Base Sepolia: SwapRouter02 `0x94cc0aac535ccdb3c01d6787d6413c739ae12bc4` and QuoterV2 `0xc5290058841028f1614f3a6f0f5816cad0df5e27`, each checked on-chain (`factory()`, `WETH9()`)

## Disclosure

The same team also submitted **[Protean-NEAR](https://github.com/replico-labs/protean-near)**. The two projects share no code.

## License

MIT, see [LICENSE](LICENSE).
