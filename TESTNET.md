

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

**Broadcast on 9 November 2026**, through the page, with the demo account's passkey:
- Cardano preprod, 5 ADA, fee 0.167789 ADA: [`66f4e383…7300a3`](https://preprod.cardanoscan.io/transaction/66f4e383673baec595d304f386572324c43383dc514c451ec53e68258b7300a3). Koios's first node answered with a submit-socket error; the relayer now retries that error.
- Zcash testnet, 0.01 ZEC with 0.0899 ZEC of change back to the account, fee 0.0001 ZEC (ZIP-317): [`e2f07643…57734297`](https://testnet.zcashexplorer.app/transactions/e2f07643ad86605804a5e16475c8fc69dbbf5bffde9d54f3bd2f042f57734297). Both outputs then showed in lightwalletd's UTXO list.

Bitcoin Cash: the demo account holds 0.02 BCH on chipnet from the BCH Ninja faucet (`bd0970e5…81ab`). The first hosted relayer answered 502 on its Electrum route. Its TLS setup used rustls's default builder, which panics when two crypto providers are linked (a test now shows it). It names ring explicitly now. Once Railway ran that build, the demo account's passkey sent 0.005 BCH through the hosted relayer, with a 452-sat fee and change back ([`f72072bb…`](https://explorer.selene.cash/tx/f72072bb04ceee882651006c517d1fe3a06ec2d1aaeb7b016ede92f4b2942e0d?network=chipnet)).

Dogecoin: built and tested against bitcoinjs-lib. QED's public testnet Esplora was down (Cloudflare 522), so nothing was broadcast.

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

**With funds (9 November 2026).**
- 50 USDC was sent to the account on Hyperliquid. It landed in spot, as all transfers between Hyperliquid users do.
- The owner moved 40 USDC to perps from the page's Hyperliquid card: one passkey tap. The page builds the `usdClassTransfer` EIP-712 digest itself, with `hyperliquidChain` "Testnet"; the digest matches hyperliquid-python-sdk. The account's EVM key signs it through Chain Signatures. Spot went from 50 to 10 USDC and perps from 0 to 40.

Then the agent, through the hosted relayer (`relayer.marvii.xyz`):
- Leverage 3x cross: accepted ([`7ZL1fv6F…`](https://testnet.nearblocks.io/txns/7ZL1fv6FVN8i7TFwUPh8EVbycZPQgGWGfB4aJ47Vgw5D)).
- Buy 0.01 ETH at 1500, Gtc: resting as order 64921782656 ([`5T4BPjof…`](https://testnet.nearblocks.io/txns/5T4BPjof1CYPefrVtRdDN6MV1FJPS77TioVKaFyUPd7G)).
- Cancel it: success ([`3ZTigCx1…`](https://testnet.nearblocks.io/txns/3ZTigCx1QzmHSdHr4MtF3cbe88VhnpbytuybbTQGbM5m)).
- Buy 0.01 ETH, Ioc at 1% over the mid: filled at 2124.77 ([`FELTSwmL…`](https://testnet.nearblocks.io/txns/FELTSwmLn1wi9LQaYPPmeu9ncmpqxz6d8jWkH99286Fi)). The account holds 0.01 ETH long, 3x cross.

A bug was found along the way: the demo agents' copy of the secp256k1 order was wrong. It was slightly too large, so a high-S signature would almost never have been normalized. Both agents now use the right constant, and the page's is checked too.

Browser tests on v4 pass:
- `e2e/testnet.mjs` now also grants Hyperliquid through the form. Confirm shows the market and leverage, the agent's order is signed, and 9x is refused.
- `e2e/move.mjs`: an account created on v3 is offered the update and moved with one tap; its address is unchanged.

## NEAR Intents (quotes)

The Swap card asks NEAR Intents' 1Click API for a quote through the relayer, which always sends `dry: true`, so no deposit address is ever created. The quote is between balances the account would hold inside Intents.

On 9 November 2026, 10 wNEAR quoted 36.14 USDC, at least 35.78 after 1% slippage, in about 10 seconds. No solver quoted wNEAR to BTC inside Intents at that moment; the page says so.

## NEAR Intents top-ups between chains (13 November 2026)

A top-up moves the account's own funds from one chain to its own address on another, through 1Click (mainnet). 1Click signs each real quote with its ed25519 key: base58(sha256(the quote's canonical JSON)), checked against `ed25519:reYaWhvwu8Jzo3WUM3zhn6VrhuMEF4eADL17qtRVifc` from 1Click's own SDK (`quote-signature.ts`). Live quotes verified against it, dry and real.

**Extension** (`extension/src/intents.rs`, action `intents_deposit`): the agent hands over the signed quote and the deposit transfer. The extension:
- checks 1Click's signature on-chain;
- requires an exact-input swap from and to the chains themselves, at most 3% slippage, and an output worth at least 95% of the input by 1Click's USD prices (a fee hidden in the request would show there);
- requires the recipient to be the account's own address on the destination chain and refunds to go to its own address on the origin chain;
- allows the quote's deposit address as a recipient for this transfer only, for at most the quoted amount of the quoted asset, under the usual caps;
- refuses a quote older than 30 minutes or one whose deposit address is about to expire.

Deposits can come from Ethereum, Base, Arbitrum, BNB Chain, Polygon and Avalanche (the coin, USDC, USDT) and Solana (SOL, USDC, USDT); destinations also include NEAR, TRON and XRP. Unit tests use real signed quotes (`extension/src/testdata/`), including a tampered one.

**Runs** (`agent/intents.mjs`, dev extension `6dd64e49…`, real 1Click quotes):
- 5 USDC on Base → SOL to the account's Solana address: checked and signed by the account's EVM key ([`WwQA2hx1…`](https://testnet.nearblocks.io/txns/WwQA2hx1u5FNepFp9BhLDPEGGDXEPWKwc1U3bNadzBL)).
- The same paying someone else's Solana address: refused, "a top-up pays only the account itself" ([`H3DvdUj6…`](https://testnet.nearblocks.io/txns/H3DvdUj6zKaxk84qc1iYNbCwkRQv4oYU64HHsjpDFxNd)).
- Depositing one unit more than quoted: refused ([`EnqQXQsy…`](https://testnet.nearblocks.io/txns/EnqQXQsy3AMuyiUmmR4DXJMYQxJvgVEQhZbqbng3Y6EH)).
- 0.002 ETH on Base → USDC on NEAR to the account: signed ([`3xf1XTwu…`](https://testnet.nearblocks.io/txns/3xf1XTwubkwDjFyARg6yEvEhkZ3cT4unFaLCLZ41SAG1)).

Not broadcast: the deposits are real mainnet transactions (the testnet MPC network signs for any EVM chain ID), and the demo account holds no mainnet funds. With funds, `node intents.mjs topup … --broadcast` sends the deposit, reports it to 1Click and follows the swap.

**Page:** Swap → "Top up another chain": a real quote paying the account's own address, then "Deposit with passkey" approves the exact transfer to the deposit address, broadcasts it from the browser and shows the swap's progress.

**Signed in OutLayer's TEE:** the module now accepts `intents_deposit` (its deposit action must itself be a known type, never another top-up). Rebuilt and published as sha256 `d6de908f…8543` (FastFS tx [`BfrRMTsk…`](https://testnet.nearblocks.io/txns/BfrRMTskSpnxytiMiYSqJ6W9GHhVfGVSLzBjHwPacA5g)); the key is bound to the build, so this module has a new TEE key, `14e5e1dc…60f4`. Granted through a permission link (key `tee-topup`, Base USDC cap only) and approved with the demo passkey:
- 5 USDC on Base → SOL to the account's Solana address, signed in the TEE and checked by the extension ([`76SZSPBB…`](https://testnet.nearblocks.io/txns/76SZSPBBFcZMMhcJZXRryQpfXyz5hiryTn4s9rr9jcoq)).
- The same paying a stranger: refused, "a top-up pays only the account itself" ([`1iKN3AvU…`](https://testnet.nearblocks.io/txns/1iKN3AvUG9JmwKA7biqVHzvgqFcCReAuyS4yWZHRSSj)).

**Relayer:** `/intents/deposit-quote` (rebuilds the exact JSON 1Click signed; a test checks it byte for byte), `/intents/submit`, `/intents/status`. None of it is trusted: the extension checks the signature itself.

## Liquid staking, Cardano, TEE-only keys and mainnet forks (13 November 2026)

Dev extension `246b7198…` (the sTRX and tsTON runs were on `57198874…`, since replaced: adding a grant field needs a fresh extension, as the final deploy will be).

**Agent keys only from Marvii's TEE.** OutLayer derives a different key per module and operator, so the proof is the run that made the key. `OutLayerSigner` records that run's NEAR transaction (`signer.keyProof`); the permission link carries it; the page reads it on NEAR and checks it called `outlayer.testnet` `request_execution` on one of Marvii's published modules with `{"operation":"public_key"}` and returned this exact key. The Add form asks for the same proof. In the browser (`e2e/connect.mjs`): a local key's link, and a link whose proof belongs to another key, were refused with the reason; the TEE key's link showed the check on Confirm and was approved. `e2e/testnet.mjs` passes (it stands in for OutLayer's lookup, since it signs with a key it holds).

**sTRX (JustLend, Nile).** `TZ8du1Hk…` is `networks.nile.strx.staked_trx` in JustLend's own `contracts.json` (STRXProxy). `deposit()`, `withdraw(uint256)` and `claimAll()` act only for the caller, so the grant lists them as TRON venues and the TRX attached counts against `trx`.
- stake 20 TRX → 5.0021 sTRX (19.999999 TRX): [`ce2ed117…`](https://nile.tronscan.org/#/transaction/ce2ed1171d3c8abed67f5b39af0cf9108bda25c4f996e2fdd7719e98689c1e44)
- unstake 5 sTRX (TRX unfreezes for the account): [`2d7110de…`](https://nile.tronscan.org/#/transaction/2d7110de4074c19123ce0e4d9a50d968114769a008e2fedee41d49127cbb73db); claim: [`b7f246ba…`](https://nile.tronscan.org/#/transaction/b7f246baff6614dad99e6003eb65c2a68770805187f42a11737ef0d91164c26a)

**tsTON (Tonstakers, TON testnet).** Pool `kQANFsYy…` from tonstakers-sdk 0.0.20 (`STAKING_CONTRACT_ADDRESS_TESTNET`); its tsTON master `kQAiQ2XK…` is the jetton master the pool administers (toncenter). The extension builds both bodies (hashes equal tonstakers-sdk's); the grant pairs each pool with its tsTON; an unstake is a burn from the account's own tsTON wallet with the account as response destination.
- stake 0.3 TON → 0.2514 tsTON: [`08fb9b7f…`](https://testnet.tonviewer.com/transaction/08fb9b7f6d18b2a74b06173d027c4e9cc89e467b3ae5e1a222b51809d9fac7ff)
- unstake 0.1 tsTON, paid out at once from the pool's liquidity: [`1d1e1825…`](https://testnet.tonviewer.com/transaction/1d1e1825dcee72301254ee3fdc24244721da80572d9248fb4d93eb7391339e5f)

**NearX (Stader, NEAR testnet).** `v2-nearx.staderlabs.testnet` (`get_nearx_price`, NearX metadata); `deposit_and_stake`, `unstake`, `withdraw_all` act for the caller (stader-labs/near-liquid-token). Register [`HSPrcKoM…`](https://testnet.nearblocks.io/txns/HSPrcKoMPGwnk2EnH9R7VLBWw84jZJJ5hYYXdaKSHQgd), stake 1.1 NEAR → 0.824 NearX [`6rrjvazN…`](https://testnet.nearblocks.io/txns/6rrjvazNDtig1SDaLQnAhmoE39rFBf9jeFJegxiJfWx8), unstake 0.1 NEAR's worth [`Bb11JaBA…`](https://testnet.nearblocks.io/txns/Bb11JaBAv1X2wmrxwhB4jtoVj8a5ia3aJ7EXsPwQX6Jf).

**Cardano agent payments (preprod).** `utxo_pay` with chain `cardano`: the extension builds the same CBOR body as core (the body hash in the tests is core's), signs its BLAKE2b-256 with the account's ed25519 key, and counts payments plus fee. Cardano only accepts a transaction whose inputs equal outputs plus the written fee, so misstated inputs can't hide a spend.
- 2 ADA to the listed address: [`f40807ff…`](https://preprod.cardanoscan.io/transaction/f40807ffa1b5568d58e790900459a2c7a15863090bfe2bc8dff2f11fea3f3552) (NEAR [`6Mtf2qyj…`](https://testnet.nearblocks.io/txns/6Mtf2qyjwTp9vtrRpPupcvP4ttpBKdyjLMA6A6s1vCYe))
- 2 ADA to a stranger: refused ([`Bdc6kC8E…`](https://testnet.nearblocks.io/txns/Bdc6kC8EGuX4vAAkZGwrGGMhvgBXxDLiN29ix9BeJP8k))

**TON NFTs.** `nft_transfer`: a TEP-62 transfer of one of the account's NFTs to a listed recipient (the body equals @ton/core's), 1 against `nft:ton`; the grant must allow NFTs. Unit-tested only: the demo account holds no testnet NFT.

**Jupiter (Surfpool fork of Solana mainnet, surfpool 1.7.0).** The decoder reads `route` and `shared_accounts_route` (Anchor discriminators from Jupiter's IDL, equal to its API's): the account signs and is the source; the output goes to its own token account; no platform fee; slippage at most 3%; the input counts against the cap. Jupiter's API built a real route for the account; the extension checked it and the fork ran it.
- 0.1 SOL → 7.959 USDC (ZeroFi): NEAR [`6peVDfhj…`](https://testnet.nearblocks.io/txns/6peVDfhjubioC5bgoSASs5p4B7h2XeoBmmFCoLATrmnq), confirmed on the fork
- output to someone else: refused ([`AbCz2A6W…`](https://testnet.nearblocks.io/txns/AbCz2A6W5uA2qhbhjTboBBeCgTn9r9NXGxtmeo9xdb17)); a platform fee: refused ([`9KSLj8SN…`](https://testnet.nearblocks.io/txns/9KSLj8SNEWrNh91UupkNA4ZzMgu9jd89y9J4sn5xmsUo))

**Phoenix spot (same fork).** Swaps (immediate-or-cancel) on markets the extension knows with their lot sizes (SOL/USDC `4DoNfFBf…`, read from its header on mainnet). Both token accounts are the account's own; a buy must be budgeted in quote lots and a sell in base lots, so what leaves the account is bounded.
- sell 0.1 SOL → 7.05 USDC (NEAR [`3YW2SvxP…`](https://testnet.nearblocks.io/txns/3YW2SvxP3zpZZNAeThy7HbKPgL3u3jJeECipWpwWkNpi)); buy with 2 USDC → 0.014 SOL, 1.87 USDC spent (NEAR [`2Fm1p9Pd…`](https://testnet.nearblocks.io/txns/2Fm1p9PdvRzfYShfxXU2tY4BeaLggdWPVdABLiLCZV1g))
- a buy budgeted in base lots: refused ([`GFY3xXzZ…`](https://testnet.nearblocks.io/txns/GFY3xXzZjtsU8iEsiA6vCxdYds1ybGhtBhfmgrSwxhie))

**Phoenix perps (same fork), the way protean-bot's Solana desk uses them.** Phoenix's perps exchange (program `EtrnLzgb…`, Ellipsis Labs' "Rise"; `agent/phoenix-perps.mjs`, extension `perps.rs`) separates two keys, and the extension holds the agent to each:
- **The account's own key** (path `solana`) is the trader accounts' authority and owns the collateral. Through `sol_tx` it may only wrap USDC into Phoenix collateral (Ember) and back, deposit into and withdraw from its own trader accounts, move collateral between them, open an isolated subaccount the grant lists (one position, rent counted), and delegate that subaccount to the account's own trading key for it (or back to itself). The USDC wrapped counts against `spl:<USDC>`. It never places orders. Phoenix's withdraw flow approves Ember's state to pull the collateral it unwraps; that approve passes only when the same transaction then unwraps exactly that amount.
- **A trading key per subaccount** (Chain Signatures, path `phoenix-<sub>`, action `phoenix_order`) is the subaccount's position authority: Phoenix lets it trade the subaccount and never withdraw. The extension signs only immediate-or-cancel orders with a limit price, on markets the grant's desk lists, up to its size per order; each order's base lots count against `phoenix:<market>`. A subaccount's losses stop at what was moved into it.

Accounts and layouts: the global configuration, perp asset map, arenas and markets are read from mainnet itself (not Phoenix's API, which this environment can't reach); instruction layouts and discriminators from `@ellipsis-labs/rise` 0.6.2, the SDK protean-bot uses. On the fork: the cross account was registered by a fork keypair (the account doesn't sign) and its capability flags set to 62, as on a live mainnet trader, standing in for Phoenix's onboarder; the script copies the market's live accounts from mainnet before each order, since the fork doesn't get oracle updates. Every other step below was checked by the extension and signed by the MPC network. Extension `0e59c2a2…`.
- deposit 40 USDC (Ember wrap, then the cross account): NEAR [`H57DxV8k…`](https://testnet.nearblocks.io/txns/H57DxV8kk4pG2A9bNSFA4QVUpzQMcjKmQFG7u4QHoVZB)
- open isolated subaccount 1, move 30 USD into it, delegate it to the trading key `D9fvHVkY…`: [`AF1ZJ8AX…`](https://testnet.nearblocks.io/txns/AF1ZJ8AXC1JBxsqPiHq3MsGM5yftZLCYemWTj1SUiCDu) (first run, on the previous dev build `80430392…`, which registered the subaccount: rent 0.00207264 SOL), [`BwRGTQ19…`](https://testnet.nearblocks.io/txns/BwRGTQ19pfK29HuKNyVnZMm2qRsWSx7HaWNjUUjJ4EGb)
- the trading key buys 0.5 SOL-PERP at up to 81.99: [`H7qVE6oW…`](https://testnet.nearblocks.io/txns/H7qVE6oWrGnh34jhk1DBeBSZoY1aycYewxiwy2kpfVyE); closes it: [`Cud7xBTb…`](https://testnet.nearblocks.io/txns/Cud7xBTbPANFAgFTZAT6f4ab6nHyMBeYgbgyKzZNzAEK)
- withdraw everything (subaccount → cross → USDC): [`91dKQuuf…`](https://testnet.nearblocks.io/txns/91dKQuufvQniH8hFVRKAD28BsCZ8B44f2MVd9swGvJpk); 40 USDC in, 39.98 back (taker fees and spread)
- refused: an order over the grant's size ([`JE2gEHsU…`](https://testnet.nearblocks.io/txns/JE2gEHsU4WRUYVW1aKVPNWvDMN3oD5MWbxXfidjqAAZ)); on BTC, which the grant doesn't list ([`EMKL8ise…`](https://testnet.nearblocks.io/txns/EMKL8ise8Rrqn57iGsvTJRiZrDSnU9y3m157QFQL39ir)); with no limit price ([`AcYmhZYi…`](https://testnet.nearblocks.io/txns/AcYmhZYiCdN4Nmk13oVfb25iV1AxPDiPGLYQEQUzMQKQ)); the trading key withdrawing ([`F9U2Gtp4…`](https://testnet.nearblocks.io/txns/F9U2Gtp4YGi6BA7F44uKTsmAztTkiuZwKQN5s4RaUu69)); the account's own key placing an order ([`GaAsRBWM…`](https://testnet.nearblocks.io/txns/GaAsRBWMReSQFqgkUCwM6NuEHpSNWvEqPo3Xp8oiPpiF)); delegating to someone else's key ([`Bo4tpgL1…`](https://testnet.nearblocks.io/txns/Bo4tpgL1AtcjNGcNcyV8ybSEuTScfgrRG5vFyNg8j8bM)); withdrawing to someone else ([`FMggc2CtC…`](https://testnet.nearblocks.io/txns/FMggc2CtCWbbALMgcJ1h43EMypU8pHciSdWLcU2fnPyT))

**Phoenix perps from an agent in OutLayer's TEE.** OutLayer module `55108592…` (it adds `phoenix_order`; uploaded in [`GBZWgepK…`](https://testnet.nearblocks.io/txns/GBZWgepKM6pLUNUvjdoEWKeLsHZ4SaKJz6qCmHbhCy46)) made key `9e14c574…` in OutLayer run [`EiwWfkXi…`](https://testnet.nearblocks.io/txns/EiwWfkXiTDu8bAsUGTyMh7uxZNiBvaTjaico56ygPLBA). A permission link asked for subaccount 1 with `{phoenix_trader_1}`, which the page filled in with the account's trading key (`D9fvHV…`), showed in plain words, and the demo passkey approved (key ID `tee-perps`, SOL-PERP up to 10 SOL an order). Orders built by `phoenix-perps.mjs` with `TEE_MODULE`, signed in the TEE, checked by the extension and signed by the MPC network:
- buy 0.2 SOL-PERP: NEAR [`2K5FeEuP…`](https://testnet.nearblocks.io/txns/2K5FeEuPqGkXSGpz6L2XCSe1ixqBurt7T1vVcY2f8HoC), confirmed on the fork; close: [`Bg1gAHyr…`](https://testnet.nearblocks.io/txns/Bg1gAHyr4vwM4UNVd29DJX7X6xhomXfWY4QcgGrsRduF), confirmed
- an earlier short 0.3 and its close ([`BFmRq29R…`](https://testnet.nearblocks.io/txns/BFmRq29RVBBgcdn2TMM3EHAD71EwAboVuhrxSmtsCCyi), [`7aBAeBqc…`](https://testnet.nearblocks.io/txns/7aBAeBqcf5B5qas3KiEfBY1weDVvvyHu9umst1QvCTZ4)) also finalized on the fork
- 20 SOL in one order: refused ([`C5pThen6…`](https://testnet.nearblocks.io/txns/C5pThen64fB8Wtfzx1QrKYgToT9EpSFGKNyg9e9RbGnJ))

On the fork, the script pauses Surfpool's clock while an order is being signed: the TEE round-trip (OutLayer, then the MPC network) can outlast a blockhash there.

**Polymarket (Anvil fork of Polygon).** `polymarket_order`: the extension builds the EIP-712 hash (equal to both exchanges' own `hashOrder` on the fork) for orders where the account is maker and signer (EOA), open to any taker, fee at most 2%, expiring within 30 days, on an exchange the grant lists. A buy counts its USDC.e; a sell counts against `polymarket:sell`. On the fork: the account approved the exchange and signed a BUY YES at 0.50 through the extension; a stand-in operator matched it with a counterparty's BUY NO; the exchange minted the shares and the account received 10 YES. (The fork's taker also collected the exchange's own leftover 3,342.70 USDC.e: `_updateTakingWithSurplus` pays the taker all it holds of that asset, and on mainnet only Polymarket's operator chooses the taker.)
- an order with someone else as maker: refused ([`7iRZZK3b…`](https://testnet.nearblocks.io/txns/7iRZZK3bqDT3mwE4mbGMxiiFEQHWbR2My73cbBckiFqo))
- signed inside OutLayer's TEE (module `02954a8c…`, key `a2625045…`, approved through a link): checked and signed ([`FpWtyttp…`](https://testnet.nearblocks.io/txns/FpWtyttpQtThrsFbuxqyNG28yZKHCyfzUsTgaCVJAFJk)); someone else as maker: refused ([`GVY9PmUs…`](https://testnet.nearblocks.io/txns/GVY9PmUsiFnK8uh9xddyVA6GS6bHBECXJVv12xhxby74))

**Not done, and why:**
- XRPL lending: `LendingProtocol` and `SingleAssetVault` are not enabled on the XRPL testnet.
- TON borrowing (EVAA): its testnet prices come from Pyth's Hermes, which now needs a key.
- Dogecoin on-chain run: QED's public Dogecoin testnet API is still down.
- Lido withdrawal #5560: Lido has finalised up to #5559.

## Extension v5: venue rules, Uniswap, payments (9 November 2026)

**Deployed** at `d591504aa95d1d7164f11dfac2016363dd18788458040c7fc3ee32cd4344e0ec` (tx [`95i8K91i…`](https://testnet.nearblocks.io/txns/95i8K91iCS7dHv85hn8wBnNrfc3n1fr1xyiWG9snuc6k)).
- The demo account's passkey moved it from v4 ([`83auiuwr…`](https://testnet.nearblocks.io/txns/83auiuwrmejz5drXJoFcJ6tf2eP455fXMAYKo9apuGsc)).
- It then granted the agent, in [`Gd7NuqdT…`](https://testnet.nearblocks.io/txns/Gd7NuqdTRW3fkSSRYxVPkJHhQvQeeRpNuP9br5jADqVq):
  - Aave V3 supply, withdraw and repay
  - Uniswap V3 swaps
  - Hyperliquid ETH perps
  - payments on Bitcoin and Zcash, to one listed address each
  - the account's own EVM address, named in the grant

**What v5 adds.**
- **Venue arguments.** For Aave `supply`, `withdraw`, `repay` and Uniswap V3 `exactInputSingle`, the extension reads where the result goes. It must be the account's own address. v4 checked only the contract and function, so an agent could have supplied or swapped for someone else, up to its cap.
- **Uniswap V3 on Base Sepolia.** SwapRouter02 `0x94cc…12bc4` and QuoterV2 `0xc529…5e27`. Their `factory()` and `WETH9()` were checked on-chain, and the USDC/WETH 0.3% pool holds liquidity.
- **Agent payments on Bitcoin, Litecoin, Bitcoin Cash and Zcash.** The extension builds the signing hashes itself, one per input. Its hashes are tested to equal core's on all four chains. Dogecoin isn't offered: its digest doesn't commit to amounts.

**On testnet**, by the agent through the relayer:
- Approved the Uniswap router for 5 USDC ([`0x9cb05d9b…`](https://sepolia.basescan.org/tx/0x9cb05d9bb4901027d83c2d68b8e9ff55a80408d403bc2ce27834e949b7c15fbc)).
- Swapped 5 USDC for at least 0.0336 WETH, output to the account ([`0xce471eb8…`](https://sepolia.basescan.org/tx/0xce471eb804b2b0e4b24b0e7b6eb3c67e5582ef41106e102278b7d82085b97793)).
- The same swap with the output to `0x…dEaD`: refused on NEAR ([`EhYuDDjo…`](https://testnet.nearblocks.io/txns/EhYuDDjoq4va9b5YRkp5aqS2QYczvrBEFdczwdAteZ7u)).
- Withdrew 10 USDC from Aave to the account ([`0x13721a60…`](https://sepolia.basescan.org/tx/0x13721a60a543b28a8de88db15db55b10b38805a0a26b1dcf90a7662309f46a82)).
- Paid 2,000 sats on Bitcoin testnet4 to the listed address, with change back and a 282-sat fee ([`89753105…`](https://mempool.space/testnet4/tx/897531058dea65e6f3ad0ec55f7c2fbc8a4c85af4bf342af0fedbd18d54cd846)).
- Paid an unlisted Bitcoin address: refused, "recipient not allowed" ([`5QPNzVR5…`](https://testnet.nearblocks.io/txns/5QPNzVR5hHm6ucvnovhHDNktW8AuRN2bwwivyLgzmf2W)).
- Paid 0.001 BTC with 2,282 sats of the 100,000-sat cap already used: refused, "over the daily cap" ([`BwV7SWT9…`](https://testnet.nearblocks.io/txns/BwV7SWT9Ea7GAWfXDtb9M6hKR6hxn6MTxwV6naEcnUkZ)).
- Paid 0.001 ZEC on Zcash testnet to the listed address ([`c7185528…`](https://testnet.zcashexplorer.app/transactions/c71855287429ee67eb7c6b67f29e970765740b0de3741aaa7898a44145cca29c)).

**A deploy mistake, fixed.** The first v5 deploy failed for lack of NEAR, after devtool had already written the new extension's key. The demo account was moved to that empty account, then straight back to v4 ([`FFsC3LPL…`](https://testnet.nearblocks.io/txns/FFsC3LPLNo3zs3EBVLN6oVxWjBJJzhSBQs2rt26zPniL)). devtool now prepares a new extension under `extension-next` and makes it current only once the contract is live.

## Agent key in a TEE: OutLayer (9 November 2026)

The demo agent's key now lives in [OutLayer](https://github.com/out-layer/outlayer), NEAR's TEE compute (Intel TDX), paid in testnet NEAR (about 0.007 NEAR a run).

- **The module** (`near/agent/outlayer/`, sha256 `4d8e953d…2d8a`) was published on FastFS (tx [`B4jwBY76…`](https://testnet.nearblocks.io/txns/B4jwBY76qHmjbfRGawM3dzV2qhsSx9J8HGrr4ySgciYX)) by the `tee-operator` account, `8f58bd66…d804`.
- **Its key**, derived inside the enclave for that build and that operator: `59af18dd0f96c837e85fd8d7ab0302ccdf81502852e958e3346f4b047e12f15f` (run [`121oaFvR…`](https://testnet.nearblocks.io/txns/121oaFvR4gnZWiepNyHcK92eeu2jqx74o2dr1ba2v5xJ)).
- **The demo account's passkey granted it** as session `tee-agent` ([`2dehpvwL…`](https://testnet.nearblocks.io/txns/2dehpvwLjWgSug9ZNp5119e1SqsadskoyhGq7MKD3MLN)), with the same Hyperliquid rules as the other agent.

Each action below is one OutLayer run (the module signs inside the TEE), then the extension on NEAR checks the rules and asks Chain Signatures to sign:

| Action | OutLayer run | Extension | Result |
|---|---|---|---|
| Leverage 3x on ETH | [`8fBvC93D…`](https://testnet.nearblocks.io/txns/8fBvC93DYmHYqmkH7D9VB6s2SHMTCv8d2kjekmafuSHF) | [`E2gvmpVy…`](https://testnet.nearblocks.io/txns/E2gvmpVyzgyLnZdwDRM35zG1PEVPE26UjEGDJo5AVFti) | Hyperliquid: ok |
| Buy 0.01 ETH at 1,900 | [`4G2Eaofy…`](https://testnet.nearblocks.io/txns/4G2EaofyW7Y7ySVPPfHyDp2c8SXgwjvS65y1mi5FZ7pW) | [`GdvT8DaQ…`](https://testnet.nearblocks.io/txns/GdvT8DaQLyb2o3if8zJ2NRAGUhBRu3Tj8Gen4cnk6uKT) | resting, oid 64929897011 |
| Cancel it | [`8bzwF99w…`](https://testnet.nearblocks.io/txns/8bzwF99wrmJYo1iLGsa5ydxfmqShc2J8a5ioyxoTpHaJ) | [`EfanM9NQ…`](https://testnet.nearblocks.io/txns/EfanM9NQ8AksGMuzzg5pF85E3VsXjYjRcuEHtWtshfz5) | success |
| Leverage 20x | [`8W1WbF7L…`](https://testnet.nearblocks.io/txns/8W1WbF7LQrUvMSBqpgSGrpmhYKiQnUWCHZGuPyyAhbkP) | [`Gff9v19T…`](https://testnet.nearblocks.io/txns/Gff9v19TCsf6kN5Dx37hidFmm6C4ZqACpnAU3REPkjGw) | refused: "leverage above what the grant allows" |

**Checking a run's quote:** `https://testnet-api.outlayer.ai/attestations/by-tx/<OutLayer run tx>`. Its report data commits to the module's sha256, the input and the output. Not fetched from this environment (its network doesn't allow that host).

**What OutLayer's own docs say is still centralised:** the coordinator is closed source, and OutLayer holds single governance keys (measurement approval, contract upgrades). The extension's rules and caps on NEAR don't depend on any of that: a compromised TEE could only sign what the session already allows.
