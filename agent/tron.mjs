// Demo agent on TRON (Nile testnet): TRX payments, SunSwap V2 swaps,
// JustLend supply / borrow / repay / redeem, and TRC-721 transfers. The agent
// sends typed fields; the extension builds the transaction, checks it and
// has the MPC network sign its ID.
//
//   node tron.mjs address                    the account and its TRX
//   node tron.mjs pay <trx> <address>        pay a listed address
//   node tron.mjs swap <trx> [to]            SunSwap: TRX for test USDT (to someone else: refused)
//   node tron.mjs supply <trx>               JustLend: supply TRX (mint jTRX)
//   node tron.mjs enter                      JustLend: use jTRX as collateral
//   node tron.mjs borrow <trx>               JustLend: borrow TRX (counted against its borrow cap)
//   node tron.mjs repay <trx>                JustLend: repay
//   node tron.mjs redeem <trx>               JustLend: take TRX back out
//   node tron.mjs nft-send <contract> <id> <to>  move one of the account's NFTs
//   node tron.mjs strx-stake <trx>           JustLend sTRX: stake TRX for sTRX (liquid staking)
//   node tron.mjs strx-unstake <strx>        burn sTRX; its TRX unfreezes (about 14 days)
//   node tron.mjs strx-claim                 claim every unfrozen withdrawal
//   node tron.mjs strx-balance               the account's sTRX and what it is worth in TRX
//
// Addresses: SunSwap V2 Router and Factory from docs.sun.io (Contract
// Addresses, NILE), the router's factory() matching; JustLend's Nile
// Unitroller from docs.justlend.org/developers/contracts.json, its markets
// from getAllMarkets() on Nile; sTRX (STRXProxy) from the same file
// (networks.nile.strx.staked_trx), its functions from its ABI on Nile.
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { recoverAddress } from 'viem';

const RELAYER = process.env.RELAYER ?? 'http://localhost:8787';
const KEYS = process.env.MARVII_NEAR_KEYS;
const WALLET = process.env.WALLET;
const KEY_ID = process.env.KEY_ID ?? 'aave-agent';
const NILE = process.env.TRON_API ?? 'https://nile.trongrid.io';
const ROUTER = 'TYMjxCXfqLpMWW1QToP6hbcjpion7EE25p';
const WTRX = 'TYsbWxNnyTgsZaTFaue9hqpxkU3Fkco94a';
const TOKEN = 'TQJ1pgmGCMdcgCg4XXFs4x6GUGft214hAU'; // a Nile test "USDT" with a WTRX pair on SunSwap V2
const JTRX = 'TKM7w4qFmkXQLEF2MgrQroBYpd5TY7i1pq';
const COMPTROLLER = 'TJUCStq3WqfKqZLuZje5v7z6Ua6iBry1P6';
const N = 0xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141n;
const STRX = 'TZ8du1HkatTWDbS6FLZei4dQfjfpSm9mxp';
const FEE_LIMIT = '50000000'; // 50 TRX of energy at most per call

function agentKey() {
  const { seed_hex } = JSON.parse(fs.readFileSync(path.join(KEYS, 'demo-agent.json'), 'utf8'));
  return crypto.createPrivateKey({ key: Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), Buffer.from(seed_hex, 'hex')]), format: 'der', type: 'pkcs8' });
}
async function relayer(p, body) {
  return (await fetch(RELAYER + p, body ? { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) } : {})).json();
}
async function view(rpc, account_id, method_name, args) {
  const r = await fetch(rpc, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'query', params: { request_type: 'call_function', finality: 'final', account_id, method_name, args_base64: Buffer.from(JSON.stringify(args)).toString('base64') } }) }).then((r) => r.json());
  if (r.error || r.result?.error) throw new Error(JSON.stringify(r.error ?? r.result.error));
  return JSON.parse(Buffer.from(r.result.result).toString());
}
async function api(p, body) {
  return (await fetch(NILE + p, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body ?? {}) })).json();
}
async function loadCore() {
  const core = await import('../web/src/core/core.js');
  await core.default({ module_or_path: fs.readFileSync(new URL('../web/src/core/core_bg.wasm', import.meta.url)) });
  return core;
}

const A = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
function hex20(t) {
  let n = 0n;
  for (const c of t) n = n * 58n + BigInt(A.indexOf(c));
  return n.toString(16).padStart(50, '0').slice(2, 42);
}
const word = (h) => h.padStart(64, '0');
const addrWord = (t) => word(hex20(t));
const pbVarint = (v) => { const o = []; v = BigInt(v); do { let b = Number(v & 0x7fn); v >>= 7n; if (v) b |= 0x80; o.push(b); } while (v); return Buffer.from(o); };

/** Has the extension build and sign the call; broadcasts it. */
async function run(call, from) {
  const cfg = await relayer('/config');
  const block = await api('/wallet/getnowblock');
  const num = BigInt(block.block_header.raw_data.number);
  const now = Date.now();
  const tx = {
    ref_block_bytes: num.toString(16).padStart(16, '0').slice(12, 16),
    ref_block_hash: block.blockID.slice(16, 32),
    expiration: String(now + 120_000), timestamp: String(now),
    fee_limit: call.kind === 'trigger' ? FEE_LIMIT : '0', call,
  };
  const s = await view(cfg.rpc, cfg.extension, 'get_session', { wallet: WALLET, key_id: KEY_ID });
  const msg = JSON.stringify({ domain: 'marvii-session-v1', extension: cfg.extension, wallet: WALLET, key_id: KEY_ID, nonce: s.nonce + 1, deadline_ms: Date.now() + 300_000, action: { type: 'tron_tx', tx } });
  const out = await relayer('/agent/execute', { msg, signature: crypto.sign(null, crypto.createHash('sha256').update(msg).digest(), agentKey()).toString('hex') });
  console.log(`  NEAR tx ${out.tx_hash ?? '-'}`);
  if (!out.ok || !out.mpc_signature) {
    console.log(`  REFUSED by the extension: ${out.reason ?? out.error ?? 'no signature'}`);
    return null;
  }
  // The extension's raw_data; the signature must recover to the account over its ID.
  const raw = Buffer.from(await view(cfg.rpc, cfg.extension, 'tron_unsigned', { wallet: WALLET, key_id: KEY_ID, tx }), 'hex');
  const id = crypto.createHash('sha256').update(raw).digest('hex');
  const sig = out.mpc_signature;
  let sv = BigInt('0x' + sig.s.scalar);
  let rec = sig.recovery_id;
  if (sv > N / 2n) { sv = N - sv; rec ^= 1; }
  const r = sig.big_r.affine_point.slice(2), sh = sv.toString(16).padStart(64, '0');
  const signer = await recoverAddress({ hash: '0x' + id, signature: { r: '0x' + r, s: '0x' + sh, yParity: rec } });
  if (signer.slice(2).toLowerCase() !== hex20(from)) throw new Error(`signature recovers to ${signer}, not ${from}`);
  const sig65 = Buffer.from(r + sh + (27 + rec).toString(16), 'hex');
  const txpb = Buffer.concat([Buffer.from([0x0a]), pbVarint(raw.length), raw, Buffer.from([0x12]), pbVarint(65), sig65]);
  const b = await api('/wallet/broadcasthex', { transaction: txpb.toString('hex') });
  console.log(`  broadcast: ${b.result ? 'ok' : JSON.stringify(b)} ${id}`);
  if (!b.result) return null;
  for (let i = 0; i < 30; i++) {
    await new Promise((r) => setTimeout(r, 3000));
    const info = await api('/wallet/gettransactioninfobyid', { value: id });
    if (info.id) {
      const ok = !info.receipt?.result || info.receipt.result === 'SUCCESS';
      console.log(`  Nile: ${ok ? 'SUCCESS' : info.receipt.result} (energy fee ${(info.receipt?.energy_fee ?? 0) / 1e6} TRX)  https://nile.tronscan.org/#/transaction/${id}`);
      return ok ? id : null;
    }
  }
  console.log('  not confirmed in 90 s');
  return null;
}

const core = await loadCore();
const cfg = await relayer('/config');
const k = await view(cfg.rpc, cfg.signer, 'derived_public_key', { path: 'tron', predecessor: WALLET, domain_id: 0 });
const from = core.chain_address('tron', k);
const sun = (x) => String(Math.round(Number(x) * 1e6));
const [cmd, a1, a2, a3] = process.argv.slice(2);
const trigger = (contract, data, call_value = '0') => ({ kind: 'trigger', contract, call_value, data: '0x' + data });

if (cmd === 'address') {
  const a = await api('/wallet/getaccount', { address: from, visible: true });
  console.log(`${from}: ${(a.balance ?? 0) / 1e6} TRX`);
} else if (cmd === 'pay') {
  console.log(`pay ${a1} TRX to ${a2}`);
  await run({ kind: 'transfer', to: a2, amount: sun(a1) }, from);
} else if (cmd === 'swap') {
  const to = a2 ?? from;
  console.log(`SunSwap: ${a1} TRX for test USDT, output to ${to}`);
  // swapExactETHForTokens(amountOutMin, path, to, deadline) 0x7ff36ab5
  const data = '7ff36ab5' + word('1') + word((128).toString(16)) + addrWord(to) + word(Math.floor(Date.now() / 1000 + 600).toString(16)) + word('2') + addrWord(WTRX) + addrWord(TOKEN);
  await run(trigger(ROUTER, data, sun(a1)), from);
} else if (cmd === 'supply') {
  console.log(`JustLend: supply ${a1} TRX`);
  await run(trigger(JTRX, '1249c58b', sun(a1)), from); // mint()
} else if (cmd === 'enter') {
  console.log('JustLend: use jTRX as collateral');
  await run(trigger(COMPTROLLER, 'c2998238' + word('20') + word('1') + addrWord(JTRX)), from); // enterMarkets([jTRX])
} else if (cmd === 'borrow') {
  console.log(`JustLend: borrow ${a1} TRX`);
  await run(trigger(JTRX, 'c5ebeaec' + word(BigInt(sun(a1)).toString(16))), from); // borrow(uint256)
} else if (cmd === 'repay') {
  console.log(`JustLend: repay ${a1} TRX`);
  await run(trigger(JTRX, '0e752702' + word(BigInt(sun(a1)).toString(16)), sun(a1)), from); // repayBorrow(uint256) payable
} else if (cmd === 'redeem') {
  console.log(`JustLend: take ${a1} TRX back out`);
  await run(trigger(JTRX, '852a12e3' + word(BigInt(sun(a1)).toString(16))), from); // redeemUnderlying(uint256)
} else if (cmd === 'strx-stake') {
  console.log(`sTRX: stake ${a1} TRX`);
  await run(trigger(STRX, 'd0e30db0', sun(a1)), from); // deposit() payable: sTRX to the caller
} else if (cmd === 'strx-unstake') {
  const amount = BigInt(Math.round(Number(a1) * 1e6)) * 10n ** 12n; // sTRX has 18 decimals
  console.log(`sTRX: unstake ${a1} sTRX (TRX unfreezes for the account)`);
  await run(trigger(STRX, '2e1a7d4d' + word(amount.toString(16))), from); // withdraw(uint256 tokenAmount)
} else if (cmd === 'strx-claim') {
  console.log('sTRX: claim unfrozen TRX');
  await run(trigger(STRX, 'd1058e59'), from); // claimAll()
} else if (cmd === 'strx-balance') {
  const q = async (sel) => BigInt('0x' + ((await api('/wallet/triggerconstantcontract', { owner_address: from, contract_address: STRX, function_selector: sel, parameter: addrWord(from), visible: true })).constant_result?.[0] || '0'));
  const bal = await q('balanceOf(address)');
  const trx = await q('viewBalanceOfUnderlying(address)');
  console.log(`${from}: ${Number(bal) / 1e18} sTRX, worth ${Number(trx) / 1e6} TRX`);
} else if (cmd === 'nft-send') {
  console.log(`send NFT ${a2} of ${a1} to ${a3}`);
  await run(trigger(a1, '42842e0e' + addrWord(from) + addrWord(a3) + word(BigInt(a2).toString(16))), from); // safeTransferFrom
} else {
  console.log('usage: see the top of tron.mjs');
}
