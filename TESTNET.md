# Testnet record (8 November 2026)

| What | Value |
|---|---|
| Wallet code (`defuse-wallet-webauthn-p256`, near/intents 32cd1f9), immutable global contract | code hash `909f39dd861604db7f75eafad948b6cf288ffc81db1fb59177030f227721c98e` (base58 `AjYXfrYzBs5BR7GmmFgCSghc6nccq4LCfSxkYTUqnZKK`), tx `BYTD6fCns4N4iWdRmh1mNW56U5JvyvCG9ZZZzo8dvkCA` |
| Session extension v3 (`near/extension`, current) | `2dc4e98d9de62211b1263a9a75d3c71caf45795719b790f12921198af309ba18`, tx `HxXn2vmU65MZ8GNtAh8EwYu6vpBebUm4uqLEVAoeUXsJ`; adds the delay queue, with a 10-minute delay for the demo |
| Session extension v2 (superseded) | `05c4922465b37c16e9d8a6b733589f83fe297ad31882d6265ba9dcebba4bfca2`, tx `CvzUEJzzr1qjcJFeBk3Xwdsay7eETykzFB3n8UtZERam`; owner the deployer, signer `v1.signer-prod.testnet`. One EVM path (`evm`) and `list_sessions` |
| Session extension v1 (superseded) | `81f602b66c298211c720fb61cd8c4959a23e610f30288c443945a9ca986c2429`, tx `HvM8WACPbzgVPpYHksUyGwK4qJGDcfiekdQynQwBP6b2`; one path per EVM chain |
| Demo wallet (created with v2, moved to v3 by its passkey, tx `HUBmzVJfPoKTNpxEaoTqJmd6vaLHJwPv9CeQgmRksxBm`; its software passkey key is kept with the testnet keys, outside the repo, so it can be funded and reused) | `0s3465557e026f471c0e84898f799c8032f356c77a`, EVM address on every EVM chain `0x48f0deef2c7e15ea9b4398a6875b43b5ab6ea81e` |
| Deployer (throwaway testnet key, not in the repo) | `89cfee7db4c52a0ef4d11f720c92060c769df650865824985d9c7951b84810cc` |

**End-to-end run with v1** (`near/devtool e2e`), all on testnet:
1. A passkey wallet (`0s978868634534dd8d2a78432b2c9121dbc9b5cada`) was created with our extension enabled. Its first request, signed by a software P-256 key in real WebAuthn format, passed the wallet's on-chain `p256_verify`.
2. The passkey granted `agent-1`: Base Sepolia, `supply` on the pool and `approve` up to 50 USDC a day (tx `Ht6KNXYVsTQrznck2SkH82riYJzgw2YTHjQw2Hy4nPZu`).
3. The agent asked for `approve(pool, 20 USDC)`. The extension checked it, and the wallet asked the MPC network to sign (tx `2FhBv2R78EZkAaMPM7K4Vca7QJP91vKQayPvgVtMpRa8`).
4. The MPC signature recovers to the wallet's derived key for path `evm-84532`, EVM address `0xe55af65fe8f321019d624fa88ff38a611ee6ee67`. The transaction was signed only, not broadcast; the pool and token addresses in this run are placeholders.

**One EVM address (decided 8 November 2026).** The wallet has one address on every EVM chain: derivation path `evm`, not one path per chain. Users expect that, as with MetaMask. Cross-chain replay is still blocked: the chain ID is inside every EIP-1559 transaction the extension hashes, and the extension never signs bare messages. Extension v2 has this. Rerun on testnet with v2: the MPC signature recovers to the wallet's derived key for `evm` (`devtool e2e`, agent tx `DmxioaZqTjMhLXX8LuKQchEqQb4ae7NiDcyDWakYWfEh`), and the browser end-to-end passes (agent tx `8YhqWiPdLKQKRSfmPa37Gt7JEDYuh7hQMnHFYuxaEmvq`). Anything that adds bare-message signing (EIP-191, EIP-712, for example Hyperliquid orders) must bind the chain or venue inside the signed data.

**Real Aave on Base Sepolia (8 November 2026).** The demo wallet's passkey granted `aave-agent` (tx `2ddTiWedyhKcYzNMN7MBAyb8EYmkaPsr3CqKCcWDwWgx`): `supply` on the Aave pool and `approve` to it, up to 50 USDC a day, for 7 days. The addresses come from Aave's address book (`bgd-labs/aave-address-book`, `AaveV3BaseSepolia.sol`) and were checked on-chain: pool `0x8bAB6d1b75f19e9eD9fCe8b9BD338844fF79aE27`, USDC `0xba50Cd2A20f6DA35D788639E581bca8d0B5d4D5f`. Circle's test USDC (`0x036C…dCF7e`) is not a reserve there, and a simulated supply of it reverts.

The demo agent (`near/agent/agent.mjs`) holds only the session key. For each transaction it builds the transaction and has the extension check it and the wallet get it signed. Before broadcasting, it checks two things: the hash the wallet signed equals viem's hash of the same transaction, and the signature recovers to the wallet's EVM address. All of these happened on Base Sepolia:
1. `approve(pool, 20 USDC)`: NEAR tx `8ZSNiD1D5QaCspTT1yc6Y75jjkLfNFsBdp6584TfFbW4`, Base Sepolia [`0x56c68d43…c06c56d`](https://sepolia.basescan.org/tx/0x56c68d432edd4b8534f3f2d975e4786d5958ac87e10fe8b0ecbafbee3c06c56d), success.
2. `supply(USDC, 20, wallet, 0)`: NEAR tx `2i6GciT7QYdoBZZJXSYKRL5PxaXB6hE3VVozH7Euwtmn`, Base Sepolia [`0x91732e77…bd98f078`](https://sepolia.basescan.org/tx/0x91732e77ab25d450c43e313ab4ec867d422b5178cfb4b555103d33e5bd98f078), success. The wallet now holds 20 aUSDC.
3. `transfer(5 USDC)` to an unknown address: refused by the extension, "recipient not allowed".
4. `approve(pool, 40 USDC)` on top of the 20 already approved that day: refused, "over the daily cap".

**Over the cap, approved with the passkey (8 November 2026).** When the extension refuses an agent's EVM request for being over the daily cap, the relayer keeps it as a pending request. The dashboard shows it under "Waiting for you", decoded on the page, with Review and Reject.

Approving builds a request from the wallet to the MPC signer's `sign`, on the wallet's `evm` path. `near/core` computes the hash from the transaction itself, using the extension's own `evm.rs` source file, so it matches the extension's hash exactly. Confirm refuses to approve if the hash in the request isn't the hash of the transaction shown. The relayer hands the signature to the agent, and the agent broadcasts only if the signature recovers to the wallet's address for its own transaction.

On Base Sepolia, with the demo wallet:
1. The agent asked to approve 40 more USDC and was refused, "over the daily cap" (NEAR tx `8Zubsd92dQR9vpqYG8DHs4LXQgs6PyohkC1rcwcgdhwT`).
2. The passkey approved it (NEAR tx `5xgZespLefF6dhcqKPuFyUFyuY4BoE2nYnKK9XoFk9hb`), and the agent broadcast it: [`0x2fb1f88a…9488de9`](https://sepolia.basescan.org/tx/0x2fb1f88a4a6bcb606739eea359cc2d62dbd63a4547c63a1d9a5a58129f488de9).
3. It then supplied the 40 USDC: [`0x6759d8ef…f637c55b`](https://sepolia.basescan.org/tx/0x6759d8efe9bb5218c73aac9b38f25f262ff25454e3d1400089dd2e30f637c55b). The wallet now holds 60 aUSDC.

The browser end-to-end covers the same path with a virtual passkey.

Note: the cap counts `approve` allowances, not the `supply` call that spends them. Together they limit what reaches the pool, because the pool can only take what was approved. A venue that pulls funds some other way (such as permit2 or an existing unlimited allowance) would need its own amount rule.

**Delay queue for new recipients and venues (extension v3, 8 November 2026).** A grant can allow `queue_new_targets`. The agent can then `propose` a payment to a new recipient or a call to a new contract. Every other rule is checked when it's proposed (signature, nonce, caps, expiry, never the wallet, the extension or the signer). The request then waits the extension's delay: 10 minutes for the demo, set at deployment, at least 1 minute, 24–48 hours for real use. Until then the passkey can `cancel_queued` it. After the delay anyone can `execute_queued` it, and the session's rules are checked again at that moment, so a revoke, a pause or a used-up cap still stops it. Each request runs once. The dashboard lists queued requests decoded, with their ready time and Cancel. 38 unit tests cover it, including attack tests: running early or twice, cancel by others, the grant not allowing it, revoke and pause, the cap checked again at execution, and calling the wallet.

Moving the demo wallet to v3 kept its address. The wallet's address comes from the extension it was created with, so the passkey added v3 and removed v2 (`demo-upgrade`). The relayer accepts wallets created with any of our extension versions.

On Base Sepolia, with the demo wallet:
1. The agent's 5 USDC transfer to a new address was refused ("recipient not allowed"), then proposed as #1 (NEAR tx `2vQANqUUttPxLQGR4duSGv1Hq8sfTcFGxVBjXMW2QwWg`). After 10 minutes it ran (NEAR tx `HaaHWjWogHUpio4uYpeR17crNWozbyH7xtG8d8zxFVWM`) and was broadcast: [`0x27413234…0d5c6213`](https://sepolia.basescan.org/tx/0x274132349a0ae5991ce7898928235e561b5c9bbe8178e561bd9ae39f0d5c6213).
2. A second proposal, #2 for 2 USDC, was cancelled by the passkey (tx `A7ndXM6P4ktPXwkMsE3BVWaSgMWWVRLnQbsQvtGyijGU`). Running it after the delay was refused: "no such queued request".

The browser end-to-end covers the same path: queue, the page shows it, the passkey cancels, none left.

**Dashboard:** the wallet's one EVM address with its balance on the testnets of every EVM chain we support:
- Base Sepolia, Ethereum Sepolia, Arbitrum Sepolia, OP Sepolia, BNB testnet, Polygon Amoy, Avalanche Fuji, Monad testnet and HyperEVM testnet
- chain IDs, RPCs and explorers from viem's chain definitions
- on Base Sepolia, Aave USDC and aUSDC as well

`?wallet=0s…` opens a read-only view of any wallet, for judges.

**Build lessons:**
- Build with `--cfg near` (cargo-near sets it). near-sdk 5.30 only uses NEAR's host functions under `cfg(near)`; without it, calls abort with an `unreachable` trap. The first wallet publish (code hash `0957bbe7…`) was built without it and is unusable; global contracts can't be deleted, and it burned about 36 testnet NEAR. The first extension (`440b33b3…ea11`) has the same fault and isn't used.
- Then run `wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int`, as cargo-near does. Bulk memory is accepted from protocol version 84 (testnet runs 88).
- Before publishing, check the wasm imports the host functions it needs (the wallet: `p256_verify`, `sha256`), then rehearse on the local sandbox (nearcore 2.14.0 `near-sandbox`, same protocol version). Global contracts burn 0.0001 NEAR per byte, so a mistake is expensive.
- The local sandbox needs its open-file limit check bypassed in this container (a small `LD_PRELOAD` shim). It has no MPC network, so a rehearsal stops at the `sign` call.

## The page and the relayer

Three parts, all in `near/`:
- **`near/core`**: the wallet's own Rust types compiled to WebAssembly for the page. The page never rebuilds the signed bytes by hand:
  - the wallet's account ID from the passkey's public key (the same NEP-616 state init as the SDK)
  - the message for a call, built with the wallet's types
  - the hash the passkey signs (`SHA3-256("NEAR_WALLET_CONTRACT/V1" || borsh(msg))`)
  - the proof: DER signature to r‖s, normalised to low S, which the wallet requires
  - a local check of the proof with the wallet's own verifier
  - the plain-words decode for Confirm, from the parsed message
  - the EVM address from the MPC signer's `derived_public_key`

  It has 9 tests, checked against the wallet's own test vectors, the SDK's account ID and the testnet wallet's EVM address. The built bindings are checked in at `near/web/src/core`, so the page builds without Rust. To rebuild them: `cargo build --release --target wasm32-unknown-unknown`, then `wasm-bindgen 0.2.130 --target web --out-name core`, then `wasm-opt -Oz`.
- **`near/relayer`**: pays gas, nothing else:
  - `/wallet/relay` relays a passkey-signed request to `w_execute_signed`, attaching the state init on first use. It relays only for wallets built from our code hash with our extension, and checks the proof first so a bad one costs no gas.
  - `/wallet/fund` gives a new wallet 0.1 testnet NEAR once, for the grant deposit.
  - `/agent/execute` forwards an agent's signed message to the extension and returns the MPC signature and the signed hash.
  - Keys come from `MARVII_NEAR_KEYS` and are never printed.
- **`near/web`**: the page (Vite, no framework):
  - create: the passkey (ES256 only), then one more tap that activates the wallet
  - the dashboard: NEAR balance, the one EVM address with its balance on Base Sepolia and Ethereum Sepolia, and agents from the extension's `list_sessions`, with Revoke
  - "Add an agent": venue, cap, expiry and inactivity
  - Confirm: decodes the exact message the passkey signs and shows any wallet op (adding or removing an extension, signature mode) as a warning

**Browser end-to-end on testnet** (`near/web/e2e/testnet.mjs`: Chromium with a virtual passkey, the relayer and the page all live):
1. The passkey created wallet `0sc1fcc635b906d31e26e5b4b2d68d75f1c8171cdc`, and activation deployed it.
2. The passkey approved "Trading agent" on the Confirm screen.
3. The agent's `approve` of 20 USDC was signed through Chain Signatures (tx `FAMdDg52ToFfTokMvSER2WzadDVQtSmx8Ax9ntHQYw1n`).
4. A further 40 USDC was refused with "over the daily cap".
5. The passkey revoked the agent, and its next request was refused with "session revoked".

The EVM addresses in the run are placeholders: it only signs, it doesn't broadcast.

**Known gaps:**
- The relayer has no rate limits; it's for the demo.
- The pending list lives in the relayer's memory, and Reject has no auth (testnet demo).

## Sending from the dashboard (owner, EVM)

Each EVM chain's dropdown has Send when the account holds something there. You pick the asset, recipient and amount, and the page builds the transaction. `near/core` builds it:
- the native coin, or ERC-20 `transfer`
- nonce and fees read from the chain

Confirm shows it decoded, and the passkey approves exactly that transaction's hash. The account then gets it signed, and the page assembles the signed transaction (`near/core`, matched byte for byte against viem). It checks the signature recovers to the account's own address before broadcasting.

On Base Sepolia, the demo account's owner sent 1 USDC to `0x…dEaD` through the page: [`0x97a6ebd8…47839691`](https://sepolia.basescan.org/tx/0x97a6ebd86e75d2f5483ea8653ac04c9ea9415d010e54f3404e3dc4c547839691), status success.

Found on the way: near-kit's `has_contract()` only checks `code_hash`. Wallets running global code (`global_contract_hash`) looked undeployed, so the relayer attached a state init on every request. That broke for a wallet created with an older extension. The relayer now checks global code too, and picks the state init that derives the wallet's own address.
