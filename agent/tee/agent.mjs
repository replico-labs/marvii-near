// The demo agent, run inside a TEE (Phala Cloud, dstack). Its session key is
// derived inside the enclave by dstack's key service and never leaves it:
// not the operator, not Phala, not us can read it. A TDX quote whose report
// data commits to the key's public half proves which code holds it (the
// app's compose hash, which pins this file by its SHA-256).
//
// The owner grants this public key a session on the Marvii extension like
// any other agent: the extension's rules still bound everything it does.
//
//   GET  /                    public key, app ID, compose hash, TDX quote
//   POST /hl/order            { side: "buy"|"sell", size, price, tif? }
//   POST /hl/leverage         { leverage }
//   POST /hl/cancel           { oid }
// POST needs `authorization: Bearer $AGENT_TOKEN` (set as an encrypted
// environment variable on Phala).
//
// Env: WALLET (the NEAR account), RELAYER (default https://relayer.marvii.xyz),
// KEY_ID (the session's key ID, default tee-agent), HL_API (default
// Hyperliquid testnet), PORT (default 8080), AGENT_TOKEN.
import crypto from 'node:crypto';
import http from 'node:http';
import { DstackClient } from '@phala/dstack-sdk';
import { getAddress, keccak256, recoverAddress } from 'viem';

const RELAYER = process.env.RELAYER ?? 'https://relayer.marvii.xyz';
const WALLET = process.env.WALLET;
const KEY_ID = process.env.KEY_ID ?? 'tee-agent';
const HL = process.env.HL_API ?? 'https://api.hyperliquid-testnet.xyz';
const TOKEN = process.env.AGENT_TOKEN ?? '';
const HL_ETH = 4; // ETH perp on Hyperliquid testnet (`meta` universe index)
const N = 0xffffffffffffffffffffffffffffffffbaaedce6af48a03bbfd25e8cd0364141n;
if (!WALLET) throw new Error('set WALLET');
if (TOKEN.length < 16) throw new Error('set AGENT_TOKEN (at least 16 characters)');

// ---- The key, from inside the enclave ----

const dstack = new DstackClient();
// dstack derives this from the app's root key in its key service: the same
// app (same compose) gets the same key after a restart; nothing else can.
const seed = (await dstack.getKey('marvii/agent/session-key', 'ed25519-seed')).key.subarray(0, 32);
const privateKey = crypto.createPrivateKey({ key: Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), Buffer.from(seed)]), format: 'der', type: 'pkcs8' });
const publicKey = crypto.createPublicKey(privateKey).export({ format: 'der', type: 'spki' }).subarray(-32).toString('hex');
// The quote's report data: SHA-256 of a label and the public key, so the
// quote can't be reused for another key.
const binding = `marvii-agent-key:${publicKey}`;
const reportData = crypto.createHash('sha256').update(binding).digest();
const quote = await dstack.getQuote(reportData);
const info = await dstack.info();
console.log(`agent key ${publicKey} for ${WALLET} (app ${info.app_id}, compose ${info.compose_hash})`);

// ---- Talking to the relayer and NEAR ----

async function relayer(p, body) {
  const r = await fetch(RELAYER + p, body ? { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) } : {});
  return r.json();
}

async function view(rpc, account_id, method_name, args) {
  const r = await fetch(rpc, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'query', params: { request_type: 'call_function', finality: 'final', account_id, method_name, args_base64: Buffer.from(JSON.stringify(args)).toString('base64') } }) }).then((r) => r.json());
  if (r.error || r.result?.error) throw new Error(JSON.stringify(r.error ?? r.result.error));
  return JSON.parse(Buffer.from(r.result.result).toString());
}

function base58(s) {
  const A = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
  let n = 0n;
  for (const c of s) n = n * 58n + BigInt(A.indexOf(c));
  let hex = n.toString(16);
  if (hex.length % 2) hex = '0' + hex;
  return Uint8Array.from(Buffer.from('00'.repeat(s.match(/^1*/)[0].length) + hex, 'hex'));
}

/** The JSON action Hyperliquid receives, in the extension's field order. */
function hlWire(a) {
  if (a.type === 'hl_order') return { type: 'order', orders: [{ a: a.asset, b: a.is_buy, p: a.price, s: a.size, r: a.reduce_only, t: { limit: { tif: a.tif } } }], grouping: 'na' };
  if (a.type === 'hl_cancel') return { type: 'cancel', cancels: [{ a: a.asset, o: a.oid }] };
  return { type: 'updateLeverage', asset: a.asset, isCross: a.is_cross, leverage: a.leverage };
}

/** Asks the extension for a signature on a Hyperliquid action; if every
 *  rule passes, checks it and sends the action to Hyperliquid. */
async function hl(action) {
  const cfg = await relayer('/config');
  const session = await view(cfg.rpc, cfg.extension, 'get_session', { wallet: WALLET, key_id: KEY_ID });
  if (!session) throw new Error(`no session ${KEY_ID} for ${WALLET}`);
  const msg = JSON.stringify({ domain: 'marvii-session-v1', extension: cfg.extension, wallet: WALLET, key_id: KEY_ID, nonce: session.nonce + 1, deadline_ms: Date.now() + 300_000, action });
  const out = await relayer('/agent/execute', { msg, signature: crypto.sign(null, crypto.createHash('sha256').update(msg).digest(), privateKey).toString('hex') });
  if (!out.ok || !out.mpc_signature) return { refused: out.reason ?? out.error ?? 'no signature', near_tx: out.tx_hash };
  const k = await view(cfg.rpc, cfg.signer, 'derived_public_key', { path: 'evm', predecessor: WALLET, domain_id: 0 });
  const from = getAddress('0x' + keccak256(base58(k.replace('secp256k1:', ''))).slice(-40));
  const digest = '0x' + (await view(cfg.rpc, cfg.extension, 'hl_signing_hash', { action }));
  const sig = out.mpc_signature;
  const r = BigInt('0x' + sig.big_r.affine_point.slice(2));
  let s = BigInt('0x' + sig.s.scalar);
  let yParity = sig.recovery_id;
  if (s > N / 2n) { s = N - s; yParity ^= 1; }
  const rs = { r: '0x' + r.toString(16).padStart(64, '0'), s: '0x' + s.toString(16).padStart(64, '0'), v: 27 + yParity };
  if ((await recoverAddress({ hash: digest, signature: { r: rs.r, s: rs.s, yParity } })) !== from) throw new Error('signature is not from the account');
  const res = await fetch(`${HL}/exchange`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ action: hlWire(action), nonce: action.nonce, signature: rs, vaultAddress: null }) }).then((r) => r.json());
  return { near_tx: out.tx_hash, account: from, hyperliquid: res };
}

// ---- HTTP ----

const authorized = (req) => {
  const got = Buffer.from(String(req.headers.authorization ?? ''));
  const want = Buffer.from(`Bearer ${TOKEN}`);
  return got.length === want.length && crypto.timingSafeEqual(got, want);
};

async function body(req) {
  let b = '';
  for await (const c of req) {
    b += c;
    if (b.length > 4096) throw new Error('too large');
  }
  return b ? JSON.parse(b) : {};
}

http
  .createServer(async (req, res) => {
    const send = (code, v) => {
      res.writeHead(code, { 'content-type': 'application/json' });
      res.end(JSON.stringify(v));
    };
    try {
      if (req.method === 'GET' && req.url === '/') {
        return send(200, { public_key: publicKey, wallet: WALLET, key_id: KEY_ID, app_id: info.app_id, compose_hash: info.compose_hash, report_data_preimage: binding, report_data: reportData.toString('hex'), quote: quote.quote });
      }
      if (req.method !== 'POST') return send(404, { error: 'not found' });
      if (!authorized(req)) return send(401, { error: 'unauthorized' });
      const b = await body(req);
      const nonce = Date.now();
      if (req.url === '/hl/order') {
        if (!['buy', 'sell'].includes(b.side)) return send(400, { error: 'side must be buy or sell' });
        return send(200, await hl({ type: 'hl_order', asset: HL_ETH, is_buy: b.side === 'buy', price: String(b.price), size: String(b.size), reduce_only: false, tif: b.tif ?? 'Gtc', nonce }));
      }
      if (req.url === '/hl/leverage') return send(200, await hl({ type: 'hl_leverage', asset: HL_ETH, is_cross: true, leverage: Number(b.leverage), nonce }));
      if (req.url === '/hl/cancel') return send(200, await hl({ type: 'hl_cancel', asset: HL_ETH, oid: Number(b.oid), nonce }));
      send(404, { error: 'not found' });
    } catch (e) {
      send(500, { error: String(e.message ?? e) });
    }
  })
  .listen(Number(process.env.PORT ?? 8080), () => console.log(`listening on ${process.env.PORT ?? 8080}`));
