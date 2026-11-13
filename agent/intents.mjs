// Demo agent: NEAR Intents top-ups. The agent moves the account's own funds
// from one chain to the account's own address on another, through 1Click.
//
//   node intents.mjs quote <amount> <from> <to>           a real 1Click quote (no deposit)
//   node intents.mjs topup <amount> <from> <to> [--broadcast]
//       gets the quote, has the extension check it and sign the deposit, and
//       with --broadcast sends it, tells 1Click and follows the swap
//   node intents.mjs topup-to <amount> <from> <to> <recipient>
//       the same, but paying someone else: refused by the extension
//   node intents.mjs topup-more <amount> <from> <to>      deposit more than quoted: refused
//   node intents.mjs status <deposit address>
//
// Assets: base-usdc, base-eth, arb-usdc, arb-eth, eth, eth-usdc, sol, sol-usdc,
// near-usdc, wnear, trx, xrp. Deposits can come from the EVM chains here (and
// Solana in the extension); any of them can be the destination.
//
// NEAR Intents runs on mainnet. The extension and MPC network are on NEAR
// testnet, but the account's EVM key signs for any chain ID, so a signed
// deposit is a real mainnet transaction: without --broadcast nothing is sent.
//
// Env as agent.mjs: MARVII_NEAR_KEYS, RELAYER, WALLET.
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { createPublicClient, http, encodeFunctionData, parseAbi, serializeTransaction, recoverTransactionAddress, formatUnits } from 'viem';
import { base, arbitrum, mainnet } from 'viem/chains';

const RELAYER = process.env.RELAYER ?? 'http://localhost:8787';
const KEYS = process.env.MARVII_NEAR_KEYS;
const WALLET = process.env.WALLET;
const KEY_ID = process.env.KEY_ID ?? 'aave-agent';
const N = 0xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141n;

// 1Click asset IDs and tokens, from its /v0/tokens (13 November 2026); the
// same table as the extension's intents.rs.
const ASSETS = {
  'base-usdc': { id: 'nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near', chain: 8453, token: '0x833589fcd6edb6e08f4c7c32d4f71b54bda02913', decimals: 6 },
  'base-eth': { id: 'nep141:base.omft.near', chain: 8453, decimals: 18 },
  'arb-usdc': { id: 'nep141:arb-0xaf88d065e77c8cc2239327c5edb3a432268e5831.omft.near', chain: 42161, token: '0xaf88d065e77c8cc2239327c5edb3a432268e5831', decimals: 6 },
  'arb-eth': { id: 'nep141:arb.omft.near', chain: 42161, decimals: 18 },
  eth: { id: 'nep141:eth.omft.near', chain: 1, decimals: 18 },
  'eth-usdc': { id: 'nep141:eth-0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48.omft.near', chain: 1, token: '0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48', decimals: 6 },
  sol: { id: 'nep141:sol.omft.near', family: 'sol', decimals: 9 },
  'sol-usdc': { id: 'nep141:sol-5ce3bf3a31af18be40ba30f721101b4341690186.omft.near', family: 'sol', decimals: 6 },
  'near-usdc': { id: 'nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1', family: 'near', decimals: 6 },
  wnear: { id: 'nep141:wrap.near', family: 'near', decimals: 24 },
  trx: { id: 'nep141:tron.omft.near', family: 'tron', decimals: 6 },
  xrp: { id: 'nep141:xrp.omft.near', family: 'xrp', decimals: 6 },
};
const EVM = {
  8453: { chain: base, rpc: process.env.BASE_RPC ?? 'https://mainnet.base.org', explorer: 'https://basescan.org/tx/' },
  42161: { chain: arbitrum, rpc: process.env.ARB_RPC ?? 'https://arb1.arbitrum.io/rpc', explorer: 'https://arbiscan.io/tx/' },
  1: { chain: mainnet, rpc: process.env.ETH_RPC ?? 'https://ethereum-rpc.publicnode.com', explorer: 'https://etherscan.io/tx/' },
};
const erc20 = parseAbi(['function transfer(address to, uint256 amount) returns (bool)', 'function balanceOf(address) view returns (uint256)']);

function agentKey() {
  const { seed_hex } = JSON.parse(fs.readFileSync(path.join(KEYS, 'demo-agent.json'), 'utf8'));
  return crypto.createPrivateKey({ key: Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), Buffer.from(seed_hex, 'hex')]), format: 'der', type: 'pkcs8' });
}
async function relayer(p, body) {
  const r = await fetch(RELAYER + p, body ? { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) } : {});
  const j = await r.json();
  if (!r.ok) throw new Error(j.error ?? j.message ?? `HTTP ${r.status}`);
  return j;
}
async function view(account_id, method_name, args) {
  const r = await fetch(cfg.rpc, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'query', params: { request_type: 'call_function', finality: 'final', account_id, method_name, args_base64: Buffer.from(JSON.stringify(args)).toString('base64') } }) }).then((r) => r.json());
  if (r.error || r.result?.error) throw new Error(JSON.stringify(r.error ?? r.result.error));
  return JSON.parse(Buffer.from(r.result.result).toString());
}
const units = (x, d) => {
  const [w, f = ''] = String(x).split('.');
  return BigInt(w) * 10n ** BigInt(d) + BigInt((f + '0'.repeat(d)).slice(0, d) || '0');
};

const cfg = await relayer('/config');
const session = await view(cfg.extension, 'get_session', { wallet: WALLET, key_id: KEY_ID });
if (!session) throw new Error(`no session ${KEY_ID} for this wallet`);
const g = session.grant;

/** The account's own address for an asset's chain, from the grant. */
function own(a) {
  const f = a.family ?? 'evm';
  const addr = { evm: g.evm_self, sol: g.sol?.self_address, near: WALLET, tron: g.tron?.self_address, xrp: g.xrpl?.self_address }[f];
  if (!addr) throw new Error(`the grant doesn't name the account's ${f} address`);
  return addr;
}
function asset(name) {
  const a = ASSETS[name];
  if (!a) throw new Error(`unknown asset ${name}; one of ${Object.keys(ASSETS).join(', ')}`);
  return a;
}

async function quote(amount, from, to, recipient) {
  const [f, t] = [asset(from), asset(to)];
  const q = await relayer('/intents/deposit-quote', { from: f.id, to: t.id, amount: units(amount, f.decimals).toString(), refund_to: own(f), recipient: recipient ?? own(t) });
  console.log(`1Click: ${amount} ${from} -> ${q.amount_out} ${to} ($${Number(q.amount_in_usd).toFixed(2)} -> $${Number(q.amount_out_usd).toFixed(2)}, ~${q.time_estimate_s}s)`);
  console.log(`  deposit address ${q.deposit_address} (until ${q.deadline}), pays ${recipient ?? own(t)}`);
  return q;
}

/** Asks the extension to check the top-up and sign the deposit. */
async function execute(action) {
  const s = await view(cfg.extension, 'get_session', { wallet: WALLET, key_id: KEY_ID });
  const msg = JSON.stringify({ domain: 'marvii-session-v1', extension: cfg.extension, wallet: WALLET, key_id: KEY_ID, nonce: s.nonce + 1, deadline_ms: Date.now() + 300_000, action });
  const out = await relayer('/agent/execute', { msg, signature: crypto.sign(null, crypto.createHash('sha256').update(msg).digest(), agentKey()).toString('hex') });
  console.log(`  NEAR tx https://testnet.nearblocks.io/txns/${out.tx_hash ?? '-'}`);
  if (!out.ok || !out.mpc_signature) {
    console.log(`  REFUSED by the extension: ${out.reason ?? out.error ?? 'no signature'}`);
    return null;
  }
  return out.mpc_signature;
}

function rsv(sig) {
  const r = BigInt('0x' + sig.big_r.affine_point.slice(2));
  let s = BigInt('0x' + sig.s.scalar);
  let yParity = sig.recovery_id;
  if (s > N / 2n) { s = N - s; yParity ^= 1; }
  return { r: '0x' + r.toString(16).padStart(64, '0'), s: '0x' + s.toString(16).padStart(64, '0'), yParity };
}

async function topup(amount, from, to, { recipient, extra = 0n, broadcast = false } = {}) {
  const f = asset(from);
  if (!f.chain) throw new Error('this script deposits from the EVM chains (base-*, arb-*, eth*)');
  const q = await quote(amount, from, to, recipient);
  const me = own(f);
  const c = createPublicClient({ chain: EVM[f.chain].chain, transport: http(EVM[f.chain].rpc) });
  const value = BigInt(q.amount_in) + extra;
  const [to_, data, val] = f.token ? [f.token, encodeFunctionData({ abi: erc20, functionName: 'transfer', args: [q.deposit_address, value] }), 0n] : [q.deposit_address, '0x', value];
  // Without a reachable RPC: placeholder nonce and fees, still checked and
  // signed by the extension, never broadcast.
  const [nonce, baseFee, prio, bal] = await Promise.all([
    c.getTransactionCount({ address: me, blockTag: 'pending' }),
    c.getBlock().then((b) => b.baseFeePerGas),
    c.estimateMaxPriorityFeePerGas(),
    f.token ? c.readContract({ address: f.token, abi: erc20, functionName: 'balanceOf', args: [me] }) : c.getBalance({ address: me }),
  ]).catch(() => {
    console.log(`  (chain ${f.chain}'s RPC is unreachable from here: placeholder nonce and fees, no broadcast)`);
    return [0, 10_000_000n, 1_000_000n, null];
  });
  if (bal !== null) console.log(`  the account ${me} holds ${formatUnits(bal, f.decimals)} ${from} on chain ${f.chain}`);
  const unsigned = { type: 'eip1559', chainId: f.chain, nonce, to: to_, value: val, data, maxPriorityFeePerGas: prio, maxFeePerGas: baseFee * 2n + prio, gas: f.token ? 100_000n : 21_000n };
  const tx = { chain_id: f.chain, nonce, max_priority_fee_per_gas: prio.toString(), max_fee_per_gas: unsigned.maxFeePerGas.toString(), gas_limit: Number(unsigned.gas), to: to_, value: val.toString(), data };
  console.log(`extension: check the quote and sign the deposit of ${formatUnits(value, f.decimals)} ${from}`);
  const sig = await execute({ type: 'intents_deposit', quote: q.quote, signature: q.signature, action: { type: 'evm_tx', tx } });
  if (!sig) return;
  const raw = serializeTransaction(unsigned, rsv(sig));
  const signer = await recoverTransactionAddress({ serializedTransaction: raw });
  console.log(`  signed by ${signer} (${signer.toLowerCase() === me.toLowerCase() ? 'the account' : 'NOT the account'})`);
  if (!broadcast) {
    console.log(`  not broadcast (add --broadcast to send it on chain ${f.chain})`);
    return;
  }
  if (bal === null || bal < value) {
    console.log('  not broadcast: the account lacks the funds');
    return;
  }
  const hash = await c.sendRawTransaction({ serializedTransaction: raw });
  console.log(`  deposit ${EVM[f.chain].explorer}${hash}`);
  await relayer('/intents/submit', { tx_hash: hash, deposit_address: q.deposit_address }).catch(() => {});
  for (let i = 0; i < 60; i++) {
    await new Promise((r) => setTimeout(r, 10_000));
    const s = await relayer(`/intents/status?deposit_address=${q.deposit_address}`);
    console.log(`  ${s.status}${s.amount_out ? ` (${s.amount_out} ${to})` : ''}`);
    if (['SUCCESS', 'REFUNDED', 'FAILED'].includes(s.status)) break;
  }
}

process.on('unhandledRejection', (e) => { console.log(`  error: ${e.shortMessage ?? e.message}`); process.exit(1); });
const [cmd, a1, a2, a3, a4] = process.argv.slice(2);
const broadcast = process.argv.includes('--broadcast');
if (cmd === 'quote') await quote(a1, a2, a3);
else if (cmd === 'topup') await topup(a1, a2, a3, { broadcast });
else if (cmd === 'topup-to') await topup(a1, a2, a3, { recipient: a4 });
else if (cmd === 'topup-more') await topup(a1, a2, a3, { extra: 1n });
else if (cmd === 'status') console.log(await relayer(`/intents/status?deposit_address=${a1}`));
else console.log('usage: see the top of intents.mjs');
