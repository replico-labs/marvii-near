

## Sending on Solana and Bitcoin (owner)

**Solana.** `near/core` builds the exact transfer message, a legacy System Program transfer, and Confirm decodes it: only that exact layout is accepted. The passkey has the account ask for an ed25519 signature over those bytes, on path `solana` with `domain_id` 1. The page builds the transaction only if the signature verifies for the account's Solana address. In the tests, the message and the signed transaction match @solana/web3.js byte for byte. On testnet, the demo account's passkey got a valid MPC signature for `8Mq6dz…3ArE` (NEAR tx `DJy2woFnPiXTcfgdS5wJCtzxZFE7yPFbWov2W3zjaKqJ`). Broadcasting waits for devnet SOL on that address.

**Bitcoin (testnet4, native SegWit).** The page reads the coins and fee rate from mempool.space. `near/core` then:
- picks coins, largest first, at most 6 inputs, with change back to the account unless it's dust
- computes one BIP-143 signing hash per input
- builds one passkey request with one signature call per input

Confirm recomputes the hashes from the plan, and refuses a request with any hash that isn't exactly the plan's. Each returned signature is matched to the input it signs and must recover to the account's key; the page then serializes the SegWit transaction. In the tests, it matches bitcoinjs-lib byte for byte (both sign RFC 6979). On testnet, a dry run with made-up coins (never broadcast) got valid signatures for both inputs from one passkey approval (NEAR tx `178dbSe51zf7cjGzgBx2CmkPxKTaRbe8FRxrTtsuFrL`). Broadcasting waits for testnet4 BTC on `tb1q9r6dyp44z2jtywrat843l505sk6ze67j4g8lay`.

**Broadcast on 8 November 2026**, through the page, with the demo account's passkey:
- Solana devnet, 0.01 SOL: [`5RfXhpLX…EiKC8Y`](https://explorer.solana.com/tx/5RfXhpLXb6SKtuUfcK19YcCGfPG6hhcSo9TzPjtu4bdZJske3t1K5Avps1zRJJYG6c6YjuqCajBVYUoomiEiKC8Y?cluster=devnet), finalized.
- Bitcoin testnet4, 10,000 sats with 345,483 sats of change back to the account and a 141-sat fee: [`208c9a11…a15b26`](https://mempool.space/testnet4/tx/208c9a113cb020cfe8923488f00d115836504b75fc59efeb52a709e178a15b26).
- TRON Nile, 5 TRX to the black-hole address, fee covered by free bandwidth: [`c19ecf98…623a79b1`](https://nile.tronscan.org/#/transaction/c19ecf9845b326898f85cfc46901356c491fb27500d75b11d9cff4c1623a79b1), block 72,547,173.
- XRP testnet, 5 XRP to the genesis account, 12-drop fee: [`A313D8EF…F3E1B871`](https://testnet.xrpl.org/transactions/A313D8EFEC71AB7C8DC4D8855982AD188FF15DF66CA5B16F8D6A0A37F3E1B871), validated in ledger 22,176,919 (9 November 2026).
- Litecoin testnet, 0.002 LTC with 799,859 litoshis of change back to the account and a 141-litoshi fee: [`6eb95764…893bc885`](https://litecoinspace.org/testnet/tx/6eb95764c7a532f8a500cb2096c6ab7a1a777e16884f7d9f40dd9b98893bc885) (9 November 2026).
- NEAR testnet, 0.01 NEAR from the demo account to another Marvii account (`0sf6db…408d`): [`A8pA7Yvt…`](https://testnet.nearblocks.io/txns/A8pA7YvtZfuooS2yVfwuEEFbEE6WfHNPpKTm8zJg9pQe) (9 November 2026).

**TRON (Nile).** The page builds the transfer itself in `near/core`: protobuf bytes from the latest block's reference. It never takes a transaction from a node, so a dishonest node can't swap the recipient. Confirm decodes the bytes strictly: it re-encodes them and requires identical bytes, so another contract type or an extra field is refused. The passkey approves a signature over the txid (sha256 of those bytes). The page broadcasts with `broadcasthex` only if the signature recovers to the account's TRON address. In the tests, the bytes, the txid and the signature match tronweb.

**XRP (testnet).** The page builds the Payment itself in `near/core`, in the ledger's canonical binary field order. The node (XRPL Labs' public testnet node, `testnet.xrpl-labs.com`, HTTPS with CORS) only supplies the account's sequence, the current ledger and the fee, and core refuses any fee over 0.1 XRP. The payment expires about 75 ledgers (around five minutes) after it's built. Confirm decodes the bytes strictly: it re-encodes them and requires identical bytes, so flags, a token amount or any extra field are refused. It shows the amount, the recipient, the fee and the destination tag if there is one. The passkey approves a signature over the signing hash (SHA-512-half of `STX\0` and those bytes). The page makes the signature low-S, as the ledger requires, and submits it only if it verifies under the transaction's own key, which must be this account's. In the tests, the unsigned and signed bytes and the transaction hash match xrpl.js 4.1.0 and ripple-keypairs, with and without a destination tag.

**Litecoin (testnet).** Litecoin uses the same plan, signing hashes (BIP-143) and transaction format as Bitcoin: the account's address prefix (`tltc1…` or `tb1…`) picks the chain, and every output must be an address on that chain (Litecoin's `Q…` P2SH form is accepted only there). Confirm also checks the signature request is on that chain's path. Change below 0.001 LTC goes to the fee: a deliberately high dust limit, since Litecoin Core's own dust rule wasn't checked. Coins, fees and broadcast come from litecoinspace.org (the same API as mempool.space). In the tests, the signed transaction and txid match bitcoinjs-lib with Litecoin testnet's parameters.

**Names (deployed 9 November 2026).**
- Registry: `marvii.testnet` (deployed [`J79AzzGA…`](https://testnet.nearblocks.io/txns/J79AzzGAqotdSdNgJjicJvsLnhKUTqba9jo38YDb1nwu)), funded with 5 NEAR; about 1.55 is held for its code's storage, and each name costs it about 0.008.
- Forwarder: immutable global contract, code hash `9428c9b6ea5401af2ecd8487170c5ceb409daa81133b8b62d70f59105d7f795e` (base58 `AyMQv6iAKoHoKJ2kWMsYKVp1Ui9EwVaxXzsUNgmbiSjF`), 27,461 bytes, published in [`AQ6kknD1…`](https://testnet.nearblocks.io/txns/AQ6kknD16SBTp9D1DgxQaDUKpWfCZ96hb3AYUEt8Xag8).
- The demo account's passkey registered `demo.marvii.testnet` ([`22pLCQkd…`](https://testnet.nearblocks.io/txns/22pLCQkdBsySC6UmeP5D5qLa9M6sqNN92BpeycmSbxEu)); 0.05 NEAR sent to the name was forwarded to the account ([`HYNEBHG4…`](https://testnet.nearblocks.io/txns/HYNEBHG4FTjsLWf6uEignv3wL5QjDmDLrSt9DVmMbPpD)).
- Rehearsed first on the local sandbox: the name account has no keys and uses 215 bytes; a second name for the same account is refused.
- Browser tests on testnet pass: `e2e/names.mjs` (a name at creation, a taken name refused, a name chosen later through Confirm) and `e2e/testnet.mjs`.

**RPC.** NEAR's own testnet RPC (`rpc.testnet.near.org`) is deprecated and now refuses requests with HTTP 429; that made account creation fail intermittently. The relayer and devtool default to FASTNEAR's (`rpc.testnet.fastnear.com`), and the page uses whatever the relayer's `/config` says.

## More chains: Cardano, Bitcoin Cash, Zcash, Dogecoin (9 November 2026)

Each is built in `near/core` and checked against the chain's own reference library on the same inputs:
- **Zcash (testnet, transparent).** v5 transactions with ZIP-244 signing hashes (BLAKE2b-256), committing to the consensus branch ID and an expiry height that lightwalletd reports (testnet is on NU7, `0x77190ad9`). zcash_primitives 0.31 parses the raw transaction, re-serializes it byte for byte, and computes the same txid and the same signing hash for each input. Fee: ZIP-317 (5,000 zatoshis per logical action, at least two).
- **Bitcoin Cash (chipnet).** BIP-143 with SIGHASH_ALL | FORKID. The raw transaction is identical to bitcore-lib-cash's, signed with the same key.
- **Dogecoin (testnet).** The original Bitcoin signing hash, version 1. Both inputs' hashes match bitcoinjs-lib's `hashForSignature`.
- **Cardano (preprod).** A Shelley-era body in CBOR (inputs, outputs, fee, TTL), signed over BLAKE2b-256 of the body by the account's ed25519 key. cardano-serialization-lib decodes it and computes the same transaction hash, and the signature verifies. Fee: 44 × size + 155,381 (preprod parameters), plus a 16-byte margin because encoders that tag sets count 6 more bytes. UTxOs carrying tokens are left alone.

Reads and broadcasts go through the relayer, under `/chain/{chain}/…`:
- Cardano: Koios.
- Zcash: lightwalletd over gRPC (`testnet.zec.rocks`).
- Bitcoin Cash: Electrum over TLS (`chipnet.bch.ninja:50002`).
- Dogecoin: an Esplora API.

Confirm decodes each plan and refuses a request whose hashes aren't exactly the plan's, as for Bitcoin.

On testnet, the demo account received 0.02 BCH on chipnet from the BCH Ninja faucet (`bd0970e5…81ab`).

## Hyperliquid (extension v4, 9 November 2026)

**Extension v4** is deployed at `cd1cd8fe1383641c975a832c077b612ef8c95935377636241d84803223f20deb` (tx [`6yHX2Nii…`](https://testnet.nearblocks.io/txns/6yHX2NiiYy9MtYnEZYVsenSHnuAdruBYjEBMUCFBxtmT)).
- The demo account's passkey moved it from v3 ([`DeqtX5qG…`](https://testnet.nearblocks.io/txns/DeqtX5qGw9hkJWsPHhFgaFSWvGBAeSkFhyNiUUT84ABE)).
- It then granted the agent ETH perps (asset 4): at most 5x, up to 100 USD of orders a day ([`9dcgc3gM…`](https://testnet.nearblocks.io/txns/9dcgc3gM8iZSYT4uimqvMHacetRw71Mx7EfDZdSjvRv1)).

The digests match hyperliquid-python-sdk for an order, a cancel and a leverage change: the MessagePack bytes and the EIP-712 digest. That vector is in the extension's tests.

On testnet, through the relayer:
- Leverage 3x: signed ([`GroCMrYt…`](https://testnet.nearblocks.io/txns/GroCMrYtCycWmMQgxSBBRZShmFEUS9vXQfroqCvMYVpf)). Hyperliquid recovered the signer as the account's own address, `0x48f0…a81e`, then answered that the account has no deposit yet.
- Leverage 10x: refused by the extension, "leverage above what the grant allows" ([`8Z57xSVY…`](https://testnet.nearblocks.io/txns/8Z57xSVY9VdoCySADQfjSWBkVyWUGAVEaymN2LAQ4bNR)).
- A 120 USD order: refused, "over the daily cap" ([`4fFx7BVe…`](https://testnet.nearblocks.io/txns/4fFx7BVeoAQaUSCfjqJZhr9YmXTaxKTb7t9ot2hyf4Ls)).
- A 20 USD order (0.01 ETH at 2000): signed ([`5PZ1T2A5…`](https://testnet.nearblocks.io/txns/5PZ1T2A57RJx7tSDAc2Fwj4Eknwvf2B3cpAMvnDZ1Lmg)). Hyperliquid again recovered the account's address.

Orders rest once the account holds testnet USDC on Hyperliquid.

Browser tests on v4 pass:
- `e2e/testnet.mjs` now also grants Hyperliquid through the form. Confirm shows the market and leverage, the agent's order is signed, and 9x is refused.
- `e2e/move.mjs`: an account created on v3 is offered the update and moved with one tap; its address is unchanged.

## TEE agent (9 November 2026)

`agent/tee/agent.mjs` derives its session key from dstack's key service and publishes a TDX quote. The quote's report data is SHA-256 of `marvii-agent-key:<public key>`.

Run locally against a stand-in for dstack (not a TEE):
- the passkey granted its key a `tee-agent` session ([`GrmrqcGc…`](https://testnet.nearblocks.io/txns/GrmrqcGcZVQuWL32gFH2nwjSQ7cJzLkWeQbYrzc71oQa))
- its ETH order was signed ([`Cz6suK7x…`](https://testnet.nearblocks.io/txns/Cz6suK7xFjcToQ8d9G9wr1rpQBzE4JNXMxUbNU8uf5Xc))
- 20x leverage was refused ([`xBRXGsY4…`](https://testnet.nearblocks.io/txns/xBRXGsY4hRk2123iSt7AvL3hZ1itQtADhWZ2cC5CH2B))
- a request without its token was refused (401)

`docker-compose.yml`'s bootstrap was also run locally: it fetches the three files, checks their SHA-256 and runs the agent. Deploying it on Phala Cloud gives the real enclave and quote.

## NEAR Intents (quotes)

The Swap card asks NEAR Intents' 1Click API for a quote through the relayer, which always sends `dry: true`, so no deposit address is ever created. The quote is between balances the account would hold inside Intents.

On 9 November 2026, 10 wNEAR quoted 36.14 USDC, at least 35.78 after 1% slippage, in about 10 seconds. No solver quoted wNEAR to BTC inside Intents at that moment; the page says so.
