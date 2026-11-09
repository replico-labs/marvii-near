# Marvii on NEAR

Marvii is a passkey wallet: self-custody secured by your phone's Face ID or fingerprint, with no seed phrase. It has one account on every chain, and agents and apps act only within limits you set.

- **Your passkey owns the account.** It's a [`defuse-wallet`](https://github.com/near/intents) contract that checks the passkey's P-256 signature on-chain (NEAR's `p256_verify`). No server holds a key that can move your funds.
- **One account, many chains.** The same account holds NEAR, has one address on every EVM chain, and has its own address on Bitcoin, Solana, TRON, XRP, Cardano, Litecoin, Dogecoin, Bitcoin Cash and Zcash, all signed through NEAR Chain Signatures.
- **Agents and apps get session keys** with rules NEAR checks on every request:
  - which contracts and functions they may call
  - who they may pay
  - a daily cap per asset
  - an expiry, and an inactivity expiry
- **Outside the rules, you decide:**
  - Over the cap, the request waits for one passkey tap.
  - New recipients and contracts wait in a delay queue, where the passkey can cancel them.
  - Revoke any agent instantly.

Testnet only. The contracts are unaudited.

## What happened on testnet

All of the following ran on NEAR testnet and each chain's testnet (8–9 November 2026). Transaction hashes, explorer links and build notes are in [TESTNET.md](TESTNET.md).

| Step | Result |
|---|---|
| The passkey grants an agent: `supply` on Aave V3, up to 50 USDC a day, 7 days | granted on NEAR |
| The agent approves 20 USDC, then supplies 20 USDC to Aave | both signed through Chain Signatures and broadcast on Base Sepolia |
| The agent asks for 40 more USDC | refused (over the daily cap), then approved by the passkey and broadcast |
| The agent pays 5 USDC to an unknown address directly | refused (recipient not allowed) |
| It proposes the same payment instead | queued for 10 minutes, then run and broadcast |
| A second proposal | cancelled by the passkey; it can never run |
| The passkey revokes the agent | its next request is refused |
| The owner sends from the dashboard: 1 USDC on Base, 0.01 SOL on Solana, 10,000 sats on Bitcoin, 0.002 LTC on Litecoin, 5 TRX on TRON, 5 XRP on XRP | each approved with the passkey, signed, broadcast |

## How it works

```
 passkey (phone) ──signs──▶ wallet contract (defuse-wallet, P-256)
                                 │  w_execute_signed: anything the owner wants
                                 │
 agent key ──signs──▶ Marvii extension ──checks rules──▶ w_execute_extension
                                                        │
                       NEAR call to a listed venue  ◀───┤
                       or Chain Signatures `sign`  ◀────┘ (path "evm": one EVM address)
                                 │
                       agent checks the signature, broadcasts the EVM transaction
```

- **`extension/`** is the session-key extension (Rust, near-sdk). The wallet's owner installs it, and it holds no funds.
  - It decodes every request itself: NEAR calls, NEP-141 transfers, and EIP-1559 transactions, from which it builds the signing hash.
  - It checks venue, recipient and cap.
  - It has the wallet act through `w_execute_extension`. The wallet itself calls the MPC signer, so the EVM address belongs to the wallet, not to the extension.
  - A session can never call the wallet, the extension or the signer directly, and never sends wallet operations.
- **`core/`** is the wallet's own Rust types compiled to WebAssembly, so the page never assembles signed bytes by hand. It builds:
  - the account ID
  - the exact hash the passkey signs
  - the WebAuthn proof
  - the EVM transaction decoding and hash, sharing the extension's `evm.rs` source
- **`relayer/`** pays gas and nothing else. It only relays for wallets built from our code, and checks the passkey's proof before spending gas. Everything it forwards is checked again on-chain.
- **`web/`** is the page: create with a passkey, balances per chain with Deposit (address and QR code) on every chain and Send on NEAR, EVM, Solana, Bitcoin, Litecoin, TRON and XRP, agents and apps, Confirm (it decodes exactly what the passkey signs), approvals, and the queue.
- **`agent/`** is a demo agent. It holds only a session key. Before broadcasting, it checks the signature recovers to the account's address for its own transaction.
- **`names/`** and **`forwarder/`** give each account a name, e.g. `alice.marvii.testnet`. NEAR wallet apps and exchanges don't yet accept `0s…` accounts (NEP-616) as recipients; a name works everywhere.
  - `names/` is the registry, deployed at `marvii.testnet`. The account calls `register("alice")` itself, so only its passkey can claim a name for it, and the page puts this in the same tap that activates the account. One name per account; a name is never reassigned.
  - The registry creates `alice.marvii.testnet` with no keys, running `forwarder/` (shared global code, published once by hash), with the account as its only target. NEAR sent to the name is forwarded by the relayer within about 20 seconds (anyone may call `forward`); tokens sent with `ft_transfer_call` are forwarded at once, and with a plain `ft_transfer` by `forward_ft`.
  - The forwarder is written against near-sdk's low-level `env` without its macros, to keep the global code small (27 KB): publishing burns NEAR per byte.
- **`devtool/`** handles deployment, a demo account and end-to-end runs.

## Deployed on NEAR testnet

| | |
|---|---|
| Wallet code (`defuse-wallet-webauthn-p256`, near/intents `32cd1f9`), immutable global contract | `909f39dd861604db7f75eafad948b6cf288ffc81db1fb59177030f227721c98e` |
| Marvii extension v3 | `2dc4e98d9de62211b1263a9a75d3c71caf45795719b790f12921198af309ba18` |
| Chain Signatures | `v1.signer-prod.testnet` |
| Names registry; forwarder code hash | `marvii.testnet`; `9428c9b6ea5401af2ecd8487170c5ceb409daa81133b8b62d70f59105d7f795e` |
| Demo account | `0s3465557e026f471c0e84898f799c8032f356c77a`, EVM `0x48f0deef2c7e15ea9b4398a6875b43b5ab6ea81e` |

## Run it

Requirements: Rust 1.97 with `wasm32-unknown-unknown`, Node 22, and a funded NEAR testnet account for the relayer.

1. **Test the contracts.** Expect 38 extension tests, 31 core tests, 6 registry tests and 8 forwarder tests to pass:
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
   MARVII_NEAR_EXTENSION=2dc4e98d9de62211b1263a9a75d3c71caf45795719b790f12921198af309ba18 \
   MARVII_NEAR_WALLET_CODE_HASH=909f39dd861604db7f75eafad948b6cf288ffc81db1fb59177030f227721c98e \
   cargo run
   ```
   You should see `relayer on 127.0.0.1:8787`. `GET /config` returns the extension and code hash.
3. **Start the page:**
   ```sh
   cd web && npm install && npm run dev
   ```
   Then open http://localhost:5173.
   - Create with a passkey, then tap once more to activate. Expect NEAR plus nine EVM chains under "Balances".
   - Add an agent or app, and approve it on the Confirm screen.
4. **See any account read-only:** http://localhost:5173/?wallet=0s3465557e026f471c0e84898f799c8032f356c77a
5. **Run the browser end-to-end** (Chromium with a virtual passkey). Expect `PASS`:
   ```sh
   cd web && node e2e/testnet.mjs
   ```
   Names, both at creation and chosen later: `node e2e/names.mjs`.

To host the relayer, see `relayer/Dockerfile`. Build it from this directory: `docker build -f relayer/Dockerfile .`.

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
- The extension's owner can pause sessions. Pausing only restricts; it can't move funds or grant anything.
- Known limits:
  - testnet only, unaudited
  - the relayer's list of requests awaiting approval lives in memory, and it has no rate limits
  - NEAR sent to a name waits on the name until forwarded (a plain transfer runs no code); the relayer forwards every 20 seconds
  - during the hackathon we keep a key to `marvii.testnet`. It can't touch existing names (they have no keys and their target is fixed), but it could redeploy the registry or create names directly; it's deleted before mainnet
  - caps count `approve` allowances; a venue that pulls funds another way would need its own amount rule
  - bare-message signing (EIP-712) isn't supported; if added, the chain must be bound inside the signed data

## Libraries

- [`defuse-wallet`](https://github.com/near/intents) and its SDK (near/intents, MIT): the passkey wallet contract
- [near-sdk](https://github.com/near/near-sdk-rs)
- [near-kit](https://crates.io/crates/near-kit)
- NEAR Chain Signatures (`v1.signer-prod.testnet`)
- [web3icons](https://github.com/0xa3k5/web3icons) (MIT): chain and token logos, copied into `web/src/icons.ts` by `web/scripts/gen-icons.mjs`
- [qrcode-generator](https://github.com/kazuhikoarase/qrcode-generator) (MIT): Deposit QR codes
- [viem](https://viem.sh): EVM transaction serialization in the demo agent
- Aave V3 Base Sepolia addresses from [bgd-labs/aave-address-book](https://github.com/bgd-labs/aave-address-book)

## Disclosure

The same team also submitted **Protean-NEAR**. The two projects share no code.

## License

MIT, see [LICENSE](LICENSE).
