# @marvii/agent-sdk

Give an AI agent (or any program) a Marvii wallet without giving it the wallet.

The agent holds only a **session key**. The wallet's owner approves what that key may do with their **passkey**, through a link to the wallet page. On every request, Marvii's extension contract on NEAR checks the action against that approval before NEAR's Chain Signatures sign it. The approval covers which contracts, which recipients, how much a day and until when. Anything else is refused, and the owner can revoke the key at any time.

```
agent ── session key signs an action ──▶ relayer ──▶ extension (checks the grant) ──▶ Chain Signatures ──▶ any chain
owner ── opens the permission link ──▶ near.marvii.xyz ──▶ passkey approves the grant
```

## Install

Node 18 or later. No dependencies for local keys. `near-api-js` is needed only if you use `OutLayerSigner` without your own `run` function.

```js
import { generateKey, LocalSigner, permissionLink, waitForApproval, MarviiAgent, nearCall } from '@marvii/agent-sdk';
```

## 1. A session key

```js
const key = generateKey();              // { seed_hex, public_key_hex }: keep seed_hex secret
const signer = new LocalSigner(key.seed_hex);
```

Or keep the key in a TEE, so that not even the agent's operator can read it:

```js
const signer = new OutLayerSigner({ moduleUrl, moduleHash, operator: { accountId, privateKey } });
const publicKey = await signer.getPublicKey();
```

`OutLayerSigner` uses Marvii's OutLayer module (`near/agent/outlayer`). The module declares one ed25519 key, bound to its own build, inside OutLayer's Intel TDX enclave. The module builds each message itself from the action's fields and never signs bytes it is handed. Each signature is an OutLayer run, paid for by `operator`, and each run has a TDX attestation.

## 2. Ask the owner

```js
const link = permissionLink({
  publicKey: signer.publicKey,
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
const { wallet } = await waitForApproval({ publicKey: signer.publicKey });
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
- Action builders: `nearCall`, `evmTx`, `solTx`, `tonTx`, `tronTx`, `stellarTx`, `xrplTx`, `utxoPay`, `seaportOrder`, `hlOrder`, `hlCancel`, `hlLeverage`, `intentsDeposit`.
- Top-ups between chains (NEAR Intents, mainnet): `agent.topUpQuote(...)` gets 1Click's signed quote; `intentsDeposit(quote, evmTx({...}))` wraps the transfer of the quoted amount to its deposit address. The extension checks 1Click's signature and that the swap pays and refunds only the wallet's own addresses, then signs. Broadcast it, then `agent.topUpSubmit(txHash, depositAddress)` and `agent.topUpStatus(depositAddress)`. `near/agent/intents.mjs` shows the whole flow. `OutLayerSigner` signs top-ups too: the module accepts `intents_deposit` only around a deposit action of a known type.
- Over the cap: the relayer returns `pending_id`. The owner can approve that exact request with the passkey, and `agent.waitForOwner(pendingId)` waits for the answer.
- A new contract or recipient: if the grant allows proposals, `agent.propose(action)` queues it behind the extension's delay, during which the owner can cancel.
- `agent.session()` returns the session; `agent.spentToday(asset)` returns what has counted against a cap today.

## Examples

- `examples/quickstart.mjs`: a local key, a link, approval, one allowed action and two refusals.
- `examples/tee-agent.mjs`: the same with the key in OutLayer's TEE.

Both were run on NEAR testnet (see `TESTNET.md`).
