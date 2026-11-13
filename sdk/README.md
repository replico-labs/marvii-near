# @replico-labs/marvii-sdk

Give an AI agent (or any program) access to your Marvii wallet without giving it the wallet keys.

The agent holds only a **session key**. The wallet's owner approves what that key may do with their **passkey**, through a link to the wallet page. On every request, Marvii's extension contract on NEAR checks the action against that approval before NEAR's Chain Signatures sign it. The approval covers which contracts, which recipients, how much a day and until when. Anything else is refused, and the owner can revoke the key at any time.

```
agent ── session key signs an action ──▶ relayer ──▶ extension (checks the grant) ──▶ Chain Signatures ──▶ any chain
owner ── opens the permission link ──▶ near.marvii.xyz ──▶ passkey approves the grant
```

## Install

Node 18 or later. No dependencies for local keys. `near-api-js` is needed only if you use `OutLayerSigner` without your own `run` function.

```js
import { OutLayerSigner, permissionLink, waitForApproval, MarviiAgent, nearCall } from '@replico-labs/marvii-sdk';
```

## 1. A session key, inside OutLayer's TEE

```js
const signer = new OutLayerSigner({ moduleUrl, moduleHash, operator: { accountId, privateKey } });
const publicKey = await signer.getPublicKey();   // also sets signer.keyProof
```

`OutLayerSigner` uses Marvii's OutLayer module (`near/agent/outlayer`, published as sha256 `02954a8c…7860`). Inside OutLayer's Intel TDX enclave, the module derives an ed25519 key bound to its own build and to `operator`; the key never leaves it. The module builds each message itself from the action's fields and never signs bytes it is handed. Each signature is an OutLayer run, paid for by `operator`, and each run has a TDX attestation.

**The wallet page approves only keys held this way.** The permission link carries `signer.keyProof`: the NEAR transaction of the OutLayer run that made the key. The page reads it on NEAR and checks that it ran Marvii's published module on `public_key` and returned this exact key. A link without it, or with a proof for another key, can't be approved.

`LocalSigner` (a key in your own process, from `generateKey()`) is for tests against your own extension and page; near.marvii.xyz won't approve it.

## 2. Ask the owner

```js
const link = permissionLink({
  publicKey,
  keyProof: signer.keyProof,           // where the key lives: checked by the page
  keyId: 'my-agent',
  app: { name: 'My agent', url: 'https://example.com' },
  days: 7,            // the key expires after this
  idleDays: 3,        // or after this long unused
  grant: {
    near_venues: [{ receiver_id: 'wrap.testnet', method: 'near_deposit' }],
    evm_venues: [{ chain_id: 84532, to: '0x8bab6d1b75f19e9ed9fce8b9bd338844ff79ae27', selector: '0x617ba037' }],
    evm_self: '{evm_self}',
    caps: [{ asset: 'near', daily: '50000000000000000000000' }],
  },
  returnUrl: 'https://example.com/connected',   // optional: the page sends the owner back with ?wallet=&key_id=
});
```

The owner opens the link on near.marvii.xyz. The page:
- fills in the wallet's own addresses (`{evm_self}`, `{sol_self}`, `{ton_self}`, `{tron_self}`, `{stellar_self}`, `{xrp_self}`, `{xrp_pk}`);
- shows every permission in plain words on its Confirm screen, together with the app's name, marked as the app's own claim;
- signs the grant with the owner's passkey.

If the request contains a part the page can't explain, approval is blocked.

`grant` takes the extension's `Grant` fields (`near/extension/src/rules.rs`); the link itself sets the key, the label and the expiry.

## 3. Learn which wallet approved

```js
const { wallet } = await waitForApproval({ publicKey });
```

The page reports the approval to the relayer. The relayer reads the session back from the extension and records only that session's own public key, so the answer can't be spoofed.

## 4. Act

```js
const agent = new MarviiAgent({ wallet, keyId: 'my-agent', signer });
const r = await agent.execute(nearCall('wrap.testnet', 'near_deposit', {}, '10000000000000000000000'));
// { ok: true, tx_hash } or { ok: false, reason: 'over the daily cap' | 'not an allowed venue' | … }
```

- NEAR calls run as the wallet.
- For other chains, `execute` returns the MPC signature over the transaction the extension built or checked. The agent then broadcasts it. `near/agent/*.mjs` shows this for EVM, Solana, TON, Stellar, TRON, XRPL, Bitcoin-style chains and Hyperliquid.
- Action builders: `nearCall`, `evmTx`, `solTx`, `tonTx`, `tronTx`, `stellarTx`, `xrplTx`, `utxoPay`, `seaportOrder`, `hlOrder`, `hlCancel`, `hlLeverage`, `intentsDeposit`, `polymarketOrder`, `phoenixOrder` (Phoenix perps orders signed by the account's trading key for an isolated subaccount; `solTx` takes `traders` for the account's own Phoenix trader accounts).
- Top-ups between chains (NEAR Intents, mainnet): `agent.topUpQuote(...)` gets 1Click's signed quote; `intentsDeposit(quote, evmTx({...}))` wraps the transfer of the quoted amount to its deposit address. The extension checks 1Click's signature and that the swap pays and refunds only the wallet's own addresses, then signs. Broadcast it, then `agent.topUpSubmit(txHash, depositAddress)` and `agent.topUpStatus(depositAddress)`. `near/agent/intents.mjs` shows the whole flow. `OutLayerSigner` signs top-ups too: the module accepts `intents_deposit` only around a deposit action of a known type.
- Over the cap: the relayer returns `pending_id`. The owner can approve that exact request with the passkey, and `agent.waitForOwner(pendingId)` waits for the answer.
- A new contract or recipient: if the grant allows proposals, `agent.propose(action)` queues it behind the extension's delay, during which the owner can cancel.
- `agent.session()` returns the session; `agent.spentToday(asset)` returns what has counted against a cap today.

## Examples

- `examples/tee-agent.mjs`: the key in OutLayer's TEE, a link, approval, one allowed action.
- `examples/quickstart.mjs`: the same flow with a local key, for testing against your own page (near.marvii.xyz refuses local keys).

Both were run on NEAR testnet (see `TESTNET.md`).
