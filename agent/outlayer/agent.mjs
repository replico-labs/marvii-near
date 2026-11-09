// The demo agent with its key inside OutLayer's TEE (Intel TDX). This script
// holds no key: it asks the OutLayer module (src/main.rs) to sign the agent's
// message, sends that to the relayer, and, once the extension's rules pass
// and Chain Signatures sign, sends the order to Hyperliquid.
//
//   WALLET=0s… node agent.mjs leverage <n>
//   WALLET=0s… node agent.mjs order <buy|sell> <size> <price> [Gtc|Ioc|Alo]
//   WALLET=0s… node agent.mjs cancel <oid>
//
// The OutLayer run goes through devtool (`outlayer-run`), which pays for it
// from the `tee-operator` account. Env: WALLET, KEY_ID (default tee-agent),
// RELAYER (default https://relayer.marvii.xyz), DEVTOOL (path to the devtool
// binary), MODULE (path to the built module), HL_API.
import { execFileSync } from 'node:child_process';
import { getAddress, keccak256, recoverAddress } from 'viem';

const RELAYER = process.env.RELAYER ?? 'https://relayer.marvii.xyz';
const WALLET = process.env.WALLET;
const KEY_ID = process.env.KEY_ID ?? 'tee-agent';
const HL = process.env.HL_API ?? 'https://api.hyperliquid-testnet.xyz';
const HL_ETH = 4; // ETH perp on Hyperliquid testnet (`meta` universe index)
const N = 0xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141n;
if (!WALLET || !process.env.DEVTOOL || !process.env.MODULE) throw new Error('set WALLET, DEVTOOL and MODULE');

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

function hlWire(a) {
  if (a.type === 'hl_order') return { type: 'order', orders: [{ a: a.asset, b: a.is_buy, p: a.price, s: a.size, r: a.reduce_only, t: { limit: { tif: a.tif } } }], grouping: 'na' };
  if (a.type === 'hl_cancel') return { type: 'cancel', cancels: [{ a: a.asset, o: a.oid }] };
  return { type: 'updateLeverage', asset: a.asset, isCross: a.is_cross, leverage: a.leverage };
}

/** One run of the module in OutLayer's TEE: returns its output. */
function outlayer(input) {
  const out = JSON.parse(execFileSync(process.env.DEVTOOL, ['outlayer-run', process.env.MODULE, JSON.stringify(input)], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] }));
  if (out.output.error) throw new Error('module: ' + out.output.error);
  console.log(`OutLayer run: NEAR tx ${out.near_tx}, module sha256 ${out.wasm_sha256}`);
  return out.output;
}

async function hl(action) {
  const cfg = await relayer('/config');
  const session = await view(cfg.rpc, cfg.extension, 'get_session', { wallet: WALLET, key_id: KEY_ID });
  if (!session) throw new Error(`no session ${KEY_ID} for ${WALLET}`);
  const signed = outlayer({ operation: 'sign', extension: cfg.extension, wallet: WALLET, key_id: KEY_ID, nonce: session.nonce + 1, action });
  console.log(`signed in the TEE by ${signed.public_key}`);
  const out = await relayer('/agent/execute', { msg: signed.msg, signature: signed.signature });
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

const [cmd, ...a] = process.argv.slice(2);
const nonce = Date.now();
let result;
if (cmd === 'key') result = outlayer({ operation: 'public_key' });
else if (cmd === 'leverage') result = await hl({ type: 'hl_leverage', asset: HL_ETH, is_cross: true, leverage: Number(a[0]), nonce });
else if (cmd === 'order') result = await hl({ type: 'hl_order', asset: HL_ETH, is_buy: a[0] === 'buy', price: a[2], size: a[1], reduce_only: false, tif: a[3] ?? 'Gtc', nonce });
else if (cmd === 'cancel') result = await hl({ type: 'hl_cancel', asset: HL_ETH, oid: Number(a[0]), nonce });
else throw new Error('usage: key | leverage <n> | order <buy|sell> <size> <price> [tif] | cancel <oid>');
console.log(JSON.stringify(result, null, 2));
