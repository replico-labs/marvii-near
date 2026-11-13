// Marvii agent SDK: give an AI agent (or any program) a Marvii wallet.
//
// The agent holds only a session key. The wallet's owner approves what that
// key may do with their passkey, on near.marvii.xyz, and Marvii's extension
// contract checks every action against that grant before NEAR's Chain
// Signatures sign it. The agent never holds the wallet's keys.
//
//   1. A session key: `generateKey()` (kept by you), or an OutLayer TEE key
//      (`OutLayerSigner`: the key never leaves the enclave).
//   2. A permission link: `permissionLink({ publicKey, keyId, app, grant })`
//      opens the wallet page, which shows the owner exactly what is asked
//      and signs the grant with their passkey.
//   3. `waitForApproval(...)` learns which wallet approved (the relayer
//      checks the grant on-chain before saying so).
//   4. `new MarviiAgent({ wallet, keyId, signer }).execute(action)`.
//
// Zero dependencies for local keys (Node 18+'s fetch and crypto);
// `OutLayerSigner` loads near-api-js when used.

import crypto from 'node:crypto';

export const DEFAULTS = {
  page: 'https://near.marvii.xyz',
  relayer: 'https://relayer.marvii.xyz',
};
const DOMAIN = 'marvii-session-v1';
/** How long a signed message stays valid (the extension accepts up to 10 minutes). */
const DEADLINE_MS = 5 * 60_000;
const PKCS8_ED25519 = Buffer.from('302e020100300506032b657004220420', 'hex');

// ---------------------------------------------------------------- keys ----

/** A new ed25519 session key: `{ seed_hex, public_key_hex }`. Keep the seed secret. */
export function generateKey() {
  const seed = crypto.randomBytes(32);
  return { seed_hex: seed.toString('hex'), public_key_hex: new LocalSigner(seed.toString('hex')).publicKey };
}

/** Signs with a session key held by this process. */
export class LocalSigner {
  /** @param {string} seedHex 32-byte ed25519 seed, hex */
  constructor(seedHex) {
    if (!/^[0-9a-fA-F]{64}$/.test(seedHex)) throw new Error('the seed must be 32 bytes of hex');
    this.key = crypto.createPrivateKey({ key: Buffer.concat([PKCS8_ED25519, Buffer.from(seedHex, 'hex')]), format: 'der', type: 'pkcs8' });
    this.publicKey = crypto.createPublicKey(this.key).export({ format: 'der', type: 'spki' }).subarray(-32).toString('hex');
  }
  /**
   * Signs a message for the extension: ed25519 over sha256(msg).
   * @param {{ extension: string, wallet: string, keyId: string, nonce: number, action: object }} m
   * @returns {Promise<{ msg: string, signature: string }>}
   */
  async signAction({ extension, wallet, keyId, nonce, action }) {
    const msg = JSON.stringify({ domain: DOMAIN, extension, wallet, key_id: keyId, nonce, deadline_ms: Date.now() + DEADLINE_MS, action });
    const signature = crypto.sign(null, crypto.createHash('sha256').update(msg).digest(), this.key).toString('hex');
    return { msg, signature };
  }
}

/**
 * Signs inside OutLayer's TEE (Intel TDX): the key is generated and kept by
 * Marvii's OutLayer module (near/agent/outlayer) and never leaves it. Each
 * signature is an OutLayer run, requested by a NEAR transaction that
 * `operator` pays for (NEAR testnet by default). The module builds the
 * message itself from the fields, so it signs only extension messages.
 *
 * `run(input)` may be supplied instead of `operator` (for example to go
 * through your own OutLayer client); it must return the module's output.
 */
export class OutLayerSigner {
  /**
   * @param {{ moduleUrl: string, moduleHash: string, operator?: { accountId: string, privateKey: string },
   *           run?: (input: object) => Promise<any>, networkId?: string, rpc?: string, outlayer?: string }} o
   */
  constructor(o) {
    this.o = { networkId: 'testnet', rpc: 'https://rpc.testnet.fastnear.com', outlayer: 'outlayer.testnet', ...o };
    this.run = o.run ?? ((input) => this.#runOnOutLayer(input));
  }
  /** The TEE key's public half (hex), for the permission link. */
  async getPublicKey() {
    if (!this.publicKey) {
      const out = await this.run({ operation: 'public_key' });
      if (!out?.public_key) throw new Error(`OutLayer: ${out?.error ?? 'no public key'}`);
      this.publicKey = out.public_key;
      // The NEAR transaction of that run: the wallet page reads it to check
      // the key came from Marvii's published module (see permissionLink).
      if (out.__run) this.keyProof = out.__run;
    }
    return this.publicKey;
  }
  async signAction({ extension, wallet, keyId, nonce, action }) {
    const out = await this.run({ operation: 'sign', extension, wallet, key_id: keyId, nonce, action });
    if (!out?.signature) throw new Error(`OutLayer: ${out?.error ?? 'no signature'}`);
    return { msg: out.msg, signature: out.signature };
  }
  async #runOnOutLayer(input) {
    const { connect, keyStores, KeyPair } = await import('near-api-js');
    const { accountId, privateKey } = this.o.operator ?? {};
    if (!accountId || !privateKey) throw new Error('OutLayerSigner needs an operator account (or a run function)');
    const keyStore = new keyStores.InMemoryKeyStore();
    await keyStore.setKey(this.o.networkId, accountId, KeyPair.fromString(privateKey));
    const near = await connect({ networkId: this.o.networkId, nodeUrl: this.o.rpc, keyStore });
    const account = await near.account(accountId);
    const text = JSON.stringify(input);
    const limits = { max_instructions: 1_000_000_000, max_memory_mb: 64, max_execution_seconds: 20 };
    const cost = await account.viewFunction({ contractId: this.o.outlayer, methodName: 'estimate_execution_cost', args: { resource_limits: limits, input_len: text.length } });
    const r = await account.functionCall({
      contractId: this.o.outlayer, methodName: 'request_execution', gas: 150_000_000_000_000n, attachedDeposit: BigInt(cost),
      args: { source: { WasmUrl: { url: this.o.moduleUrl, hash: this.o.moduleHash, build_target: 'wasm32-wasip2' } }, resource_limits: limits, input_data: text, response_format: 'Json' },
    });
    const v = r.status?.SuccessValue;
    if (v === undefined) throw new Error(`OutLayer run failed: ${JSON.stringify(r.status)}`);
    const out = JSON.parse(Buffer.from(v, 'base64').toString());
    if (input.operation === 'public_key') out.__run = { tx_hash: r.transaction.hash, sender: accountId };
    return out;
  }
}

// ------------------------------------------------------ permission link ----

/**
 * The link the owner opens to approve the agent. The page fills in the
 * wallet's own addresses (`{evm_self}`, `{sol_self}`, `{ton_self}`, …, and
 * `{phoenix_trader_<n>}` for its Phoenix perps trading key of subaccount n),
 * shows every permission in plain words, and signs the grant with the
 * owner's passkey. Nothing in the link can grant more than the page shows.
 *
 * @param {{ publicKey: string, keyId: string, app: { name: string, url?: string },
 *           grant: object, days?: number, idleDays?: number, returnUrl?: string, page?: string,
 *           keyProof?: { tx_hash: string, sender: string } }} r
 *   `keyProof`: `signer.keyProof` from an OutLayerSigner. The wallet page
 *   approves only keys made inside Marvii's published OutLayer module, and
 *   checks this run on NEAR to tell.
 *   `grant`: the extension's Grant fields (venues, recipients, caps, chain
 *   sections), without public_key, label or expiry: the link sets those.
 * @returns {string}
 */
export function permissionLink({ publicKey, keyId, app, grant, days = 7, idleDays = 3, returnUrl, page = DEFAULTS.page, keyProof }) {
  if (!/^[0-9a-f]{64}$/.test(publicKey)) throw new Error('publicKey: 32 bytes of lowercase hex');
  if (!/^[a-z0-9_.-]{1,64}$/.test(keyId)) throw new Error('keyId: up to 64 of a-z 0-9 _ . -');
  if (!app?.name) throw new Error('app.name is required');
  for (const k of ['public_key', 'label', 'expires_at_ms', 'inactivity_ms']) if (k in grant) throw new Error(`grant.${k} is set by the link`);
  if (keyProof && !(/^[1-9A-HJ-NP-Za-km-z]{43,44}$/.test(keyProof.tx_hash) && keyProof.sender)) throw new Error('keyProof: { tx_hash, sender } from OutLayerSigner');
  const request = { v: 1, app: { name: app.name, url: app.url }, key_id: keyId, public_key: publicKey, days, idle_days: idleDays, grant, return_url: returnUrl, key_proof: keyProof };
  return `${page.replace(/\/$/, '')}/#/connect?r=${Buffer.from(JSON.stringify(request)).toString('base64url')}`;
}

/**
 * Waits until a wallet has granted `publicKey` (the page reports it to the
 * relayer, which checks the grant on-chain). Returns `{ wallet, key_id }`.
 */
export async function waitForApproval({ publicKey, relayer = DEFAULTS.relayer, timeoutMs = 15 * 60_000, intervalMs = 3000 }) {
  const until = Date.now() + timeoutMs;
  while (Date.now() < until) {
    const r = await fetch(`${relayer}/connect/${publicKey}`);
    if (r.ok) return r.json();
    await new Promise((res) => setTimeout(res, intervalMs));
  }
  throw new Error('not approved in time');
}

// --------------------------------------------------------------- agent ----

/** An agent acting for one wallet through one session key. */
export class MarviiAgent {
  /** @param {{ wallet: string, keyId: string, signer: LocalSigner | OutLayerSigner, relayer?: string }} o */
  constructor({ wallet, keyId, signer, relayer = DEFAULTS.relayer }) {
    Object.assign(this, { wallet, keyId, signer, relayer });
  }
  async #config() {
    this.cfg ??= await this.#get('/config');
    return this.cfg;
  }
  async #get(p) {
    const r = await fetch(this.relayer + p);
    return r.json();
  }
  async #post(p, body) {
    const r = await fetch(this.relayer + p, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });
    return r.json();
  }
  /** A view call on NEAR (through the relayer's RPC). */
  async view(contractId, method, args = {}) {
    const cfg = await this.#config();
    const r = await fetch(cfg.rpc, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'query', params: { request_type: 'call_function', finality: 'final', account_id: contractId, method_name: method, args_base64: Buffer.from(JSON.stringify(args)).toString('base64') } }) }).then((r) => r.json());
    if (r.error || r.result?.error) throw new Error(JSON.stringify(r.error ?? r.result.error));
    return JSON.parse(Buffer.from(r.result.result).toString());
  }
  /** The session as the extension holds it: grant, nonce, spending, revoked. */
  async session() {
    const cfg = await this.#config();
    return this.view(cfg.extension, 'get_session', { wallet: this.wallet, key_id: this.keyId });
  }
  /** What has counted against `asset`'s daily cap today. */
  async spentToday(asset) {
    const cfg = await this.#config();
    return BigInt(await this.view(cfg.extension, 'spent_today', { wallet: this.wallet, key_id: this.keyId, asset }));
  }
  /** The wallet's address on a chain (Chain Signatures path), e.g. `evm`. */
  async address(path) {
    const s = await this.session();
    const g = s?.grant ?? {};
    return { evm: g.evm_self, solana: g.sol?.self_address, ton: g.ton?.self_address, stellar: g.stellar?.self_address, tron: g.tron?.self_address, xrpl: g.xrpl?.self_address }[path];
  }
  async #signed(action) {
    const cfg = await this.#config();
    const s = await this.session();
    if (!s) throw new Error(`no session ${this.keyId} on ${this.wallet}`);
    return this.signer.signAction({ extension: cfg.extension, wallet: this.wallet, keyId: this.keyId, nonce: s.nonce + 1, action });
  }

  /**
   * Asks the extension to perform `action` (see the action builders below).
   * Resolves to the relayer's answer:
   *   `{ ok: true, tx_hash, mpc_signature? }`: done (NEAR calls) or signed
   *     (other chains: broadcast with the signature);
   *   `{ ok: false, reason }`: refused by the extension (or failed);
   *   `{ ok: false, pending_id }`: over the cap; the owner can approve it
   *     with the passkey (`waitForOwner`).
   */
  async execute(action) {
    return this.#post('/agent/execute', await this.#signed(action));
  }
  /**
   * Proposes an action to a recipient or contract the grant doesn't list
   * (if the grant allows proposals): it waits out the extension's delay,
   * during which the owner can cancel it. Resolves when it has run.
   */
  async propose(action, { wait = true } = {}) {
    const p = await this.#post('/agent/propose', await this.#signed(action));
    if (!p.queue_id || !wait) return p;
    await new Promise((r) => setTimeout(r, Math.max(0, p.ready_at_ms - Date.now()) + 5000));
    return this.#post(`/queued/${p.queue_id}/execute`, { wallet: this.wallet });
  }
  /**
   * A NEAR Intents top-up quote (1Click, mainnet): `from` and `to` are 1Click
   * asset IDs, `amount` in the origin asset's base units, `refundTo` and
   * `recipient` the wallet's own addresses on the two chains. Returns the
   * signed quote (`quote`, `signature`) for `intentsDeposit`, the
   * `deposit_address` and the amounts.
   */
  async topUpQuote({ from, to, amount, refundTo, recipient, slippageBps }) {
    return this.#post('/intents/deposit-quote', { from, to, amount: String(amount), refund_to: refundTo, recipient, slippage_bps: slippageBps });
  }
  /** Tells 1Click the deposit was sent, then returns the swap's status. */
  async topUpSubmit(txHash, depositAddress) {
    return this.#post('/intents/submit', { tx_hash: txHash, deposit_address: depositAddress });
  }
  async topUpStatus(depositAddress) {
    return this.#get(`/intents/status?deposit_address=${encodeURIComponent(depositAddress)}`);
  }
  /** Waits for the owner to approve or reject an over-the-cap request. */
  async waitForOwner(pendingId, { timeoutMs = 10 * 60_000 } = {}) {
    const until = Date.now() + timeoutMs;
    while (Date.now() < until) {
      const p = await this.#get(`/pending/${pendingId}`);
      if (p.status === 'approved' || p.status === 'rejected') return p;
      await new Promise((r) => setTimeout(r, 3000));
    }
    throw new Error('the owner did not answer in time');
  }
}

// ------------------------------------------------------ action builders ----

/** A call from the wallet to a NEAR contract; `deposit` in yoctoNEAR. */
export const nearCall = (receiverId, method, args = {}, deposit = '0') => ({ type: 'near_call', receiver_id: receiverId, method, args: JSON.stringify(args), deposit: String(deposit) });
/** An EIP-1559 transaction for the wallet's EVM address (all fields as the chain needs them). */
export const evmTx = (tx) => ({ type: 'evm_tx', tx: { value: '0', data: '0x', ...tx, value: String(tx.value ?? '0') } });
/** Solana instructions (see near/agent/solana.mjs); the extension compiles the message. */
export const solTx = ({ recentBlockhash, instructions, atas = [], obligations = [], traders = [] }) => ({ type: 'sol_tx', recent_blockhash: recentBlockhash, instructions, atas, obligations, traders });
/** Phoenix perps orders signed by the account's trading key for isolated subaccount `sub` (`bump`: its trader account's PDA bump). */
export const phoenixOrder = ({ sub, bump, recentBlockhash, instructions }) => ({ type: 'phoenix_order', sub, bump, recent_blockhash: recentBlockhash, instructions });
/** Typed transactions the extension builds itself. */
export const tonTx = (tx) => ({ type: 'ton_tx', tx });
export const tronTx = (tx) => ({ type: 'tron_tx', tx });
export const stellarTx = (tx) => ({ type: 'stellar_tx', tx });
export const xrplTx = (tx) => ({ type: 'xrpl_tx', tx });
/** A Polymarket order (Polygon) for the wallet's EVM key to sign (EIP-712); the account is maker and signer. */
export const polymarketOrder = (order) => ({ type: 'polymarket_order', order });
/** A Seaport listing for the wallet's EVM key to sign (EIP-712). */
export const seaportOrder = (order) => ({ type: 'seaport_order', order });
/** Hyperliquid (testnet) order, cancel and leverage. */
export const hlOrder = (o) => ({ type: 'hl_order', ...o });
export const hlCancel = (o) => ({ type: 'hl_cancel', ...o });
export const hlLeverage = (o) => ({ type: 'hl_leverage', ...o });
/**
 * A NEAR Intents top-up: 1Click's signed quote (from `agent.topUpQuote`) and
 * the deposit transfer (an `evmTx` or `solTx` paying `deposit_address` the
 * quoted amount). The extension checks the signature and that the swap pays
 * and refunds only the wallet itself.
 */
export const intentsDeposit = ({ quote, signature }, action) => ({ type: 'intents_deposit', quote, signature, action });
/** A payment on a Bitcoin-style chain: `{ chain, inputs, outputs, branch_id?, expiry_height? }`. */
export const utxoPay = (p) => ({ type: 'utxo_pay', ...p });
