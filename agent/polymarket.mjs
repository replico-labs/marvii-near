// Demo agent: a Polymarket order signed through the extension and settled on
// an Anvil fork of Polygon. Polymarket's CLOB matches orders off-chain and
// its operator settles them on the CTF exchange; on the fork, a test account
// stands in as operator (its role set in the exchange's storage) and another
// as the counterparty.
//
//   node polymarket.mjs settle <usdc> <yes token> <no token>
//       (starts its own Anvil fork once the extension has signed: the public
//       RPC serves only recent state, so the fork must be fresh)
//       the account buys YES at 0.50 (its order signed through the
//       extension), a counterparty buys NO at 0.50, the operator matches
//       them: the exchange splits both payments into YES and NO shares
//   node polymarket.mjs order-to <usdc> <yes token> <address>
//       an order with someone else as maker (refused)
//
// Env as agent.mjs: MARVII_NEAR_KEYS, RELAYER, WALLET; FORK_RPC (default
// http://127.0.0.1:8546).
import crypto from 'node:crypto';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { createPublicClient, createWalletClient, http, parseAbi, encodeFunctionData, serializeTransaction, keccak256, encodeAbiParameters, pad, toHex, recoverAddress } from 'viem';
import { privateKeyToAccount, generatePrivateKey } from 'viem/accounts';
import { polygon } from 'viem/chains';

const RELAYER = process.env.RELAYER ?? 'http://localhost:8787';
const KEYS = process.env.MARVII_NEAR_KEYS;
const WALLET = process.env.WALLET;
const KEY_ID = process.env.KEY_ID ?? 'aave-agent';
const RPC = process.env.FORK_RPC ?? 'http://127.0.0.1:8546';
const N = 0xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141n;
// Polymarket's CTF exchange, its collateral (USDC.e) and the conditional
// tokens contract: getCollateral() and getCtf() on the exchange.
const EXCHANGE = '0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E';
const USDC_E = '0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174';
const CTF = '0x4D97DCd97eC945f40cF65F87097ACe5EA0476045';
// Fork-only stand-ins, new keys each run (Anvil's well-known test
// accounts carry code on Polygon now, which refuses ERC-1155 transfers).
const COUNTERPARTY = privateKeyToAccount(generatePrivateKey());
const OPERATOR = privateKeyToAccount(generatePrivateKey());

const abi = parseAbi([
  'function approve(address spender, uint256 amount) returns (bool)',
  'function balanceOf(address) view returns (uint256)',
  'function balanceOf(address owner, uint256 id) view returns (uint256)',
  'function setApprovalForAll(address operator, bool approved)',
  'function isOperator(address) view returns (bool)',
  'function getComplement(uint256 token) view returns (uint256)',
  'function matchOrders((uint256 salt, address maker, address signer, address taker, uint256 tokenId, uint256 makerAmount, uint256 takerAmount, uint256 expiration, uint256 nonce, uint256 feeRateBps, uint8 side, uint8 signatureType, bytes signature) takerOrder, (uint256 salt, address maker, address signer, address taker, uint256 tokenId, uint256 makerAmount, uint256 takerAmount, uint256 expiration, uint256 nonce, uint256 feeRateBps, uint8 side, uint8 signatureType, bytes signature)[] makerOrders, uint256 takerFillAmount, uint256[] makerFillAmounts)',
]);
// The exchange's custom errors (ctf-exchange interfaces), so a revert is readable.
const ERRORS = parseAbi(['error NotOperator()', 'error NotCrossing()', 'error InvalidSignature()', 'error OrderExpired()', 'error InvalidNonce()', 'error OrderFilledOrCancelled()', 'error MismatchedTokenIds()', 'error TooLittleTokensReceived()', 'error NotTaker()', 'error FeeTooHigh()', 'error Paused()', 'error MakingGtRemaining()', 'error InvalidComplement()', 'error NotOwner()', 'error InvalidTokenId()']);
const ORDER_TYPES = {
  Order: [
    { name: 'salt', type: 'uint256' }, { name: 'maker', type: 'address' }, { name: 'signer', type: 'address' }, { name: 'taker', type: 'address' },
    { name: 'tokenId', type: 'uint256' }, { name: 'makerAmount', type: 'uint256' }, { name: 'takerAmount', type: 'uint256' }, { name: 'expiration', type: 'uint256' },
    { name: 'nonce', type: 'uint256' }, { name: 'feeRateBps', type: 'uint256' }, { name: 'side', type: 'uint8' }, { name: 'signatureType', type: 'uint8' },
  ],
};
const DOMAIN = { name: 'Polymarket CTF Exchange', version: '1', chainId: 137, verifyingContract: EXCHANGE };
const Z = '0x0000000000000000000000000000000000000000';

const FORK_URL = process.env.FORK_URL ?? 'https://polygon-bor-rpc.publicnode.com';
const fork = createPublicClient({ chain: polygon, transport: http(RPC) });
/** A fresh Anvil fork of Polygon at the latest block. */
async function startFork() {
  const p = spawn(process.env.ANVIL ?? 'anvil', ['--fork-url', FORK_URL, '--chain-id', '137', '--port', new URL(RPC).port, '--silent'], { stdio: 'ignore' });
  for (let i = 0; i < 60; i++) {
    await new Promise((r) => setTimeout(r, 500));
    if (await fork.getChainId().then(() => true, () => false)) return p;
  }
  throw new Error('anvil did not start');
}
const rpc = (method, params) => fork.request({ method, params });

function agentKey() {
  const { seed_hex } = JSON.parse(fs.readFileSync(path.join(KEYS, 'demo-agent.json'), 'utf8'));
  return crypto.createPrivateKey({ key: Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), Buffer.from(seed_hex, 'hex')]), format: 'der', type: 'pkcs8' });
}
async function relayer(p, body) {
  return (await fetch(RELAYER + p, body ? { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) } : {})).json();
}
async function view(account_id, method_name, args) {
  const r = await fetch(cfg.rpc, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'query', params: { request_type: 'call_function', finality: 'final', account_id, method_name, args_base64: Buffer.from(JSON.stringify(args)).toString('base64') } }) }).then((r) => r.json());
  if (r.error || r.result?.error) throw new Error(JSON.stringify(r.error ?? r.result.error));
  return JSON.parse(Buffer.from(r.result.result).toString());
}
const cfg = await relayer('/config');
const session = await view(cfg.extension, 'get_session', { wallet: WALLET, key_id: KEY_ID });
const ME = session.grant.evm_self;

async function execute(action) {
  const s = await view(cfg.extension, 'get_session', { wallet: WALLET, key_id: KEY_ID });
  const msg = JSON.stringify({ domain: 'marvii-session-v1', extension: cfg.extension, wallet: WALLET, key_id: KEY_ID, nonce: s.nonce + 1, deadline_ms: Date.now() + 300_000, action });
  const out = await relayer('/agent/execute', { msg, signature: crypto.sign(null, crypto.createHash('sha256').update(msg).digest(), agentKey()).toString('hex') });
  console.log(`  NEAR tx ${out.tx_hash ?? '-'}`);
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

/** Gives `who` `amount` of an ERC-20 on the fork by finding its balance slot. */
async function fund(token, who, amount) {
  for (let slot = 0n; slot < 30n; slot++) {
    const key = keccak256(encodeAbiParameters([{ type: 'address' }, { type: 'uint256' }], [who, slot]));
    const before = await rpc('eth_getStorageAt', [token, key, 'latest']);
    await rpc('anvil_setStorageAt', [token, key, pad(toHex(amount))]);
    if ((await fork.readContract({ address: token, abi, functionName: 'balanceOf', args: [who] })) === amount) return;
    await rpc('anvil_setStorageAt', [token, key, before]);
  }
  throw new Error('balance slot not found');
}
/** Makes `who` an operator of the exchange on the fork (the role's mapping slot found the same way). */
async function makeOperator(who) {
  for (let slot = 0n; slot < 30n; slot++) {
    const key = keccak256(encodeAbiParameters([{ type: 'address' }, { type: 'uint256' }], [who, slot]));
    await rpc('anvil_setStorageAt', [EXCHANGE, key, pad(toHex(1n))]);
    if (await fork.readContract({ address: EXCHANGE, abi, functionName: 'isOperator', args: [who] })) return;
    await rpc('anvil_setStorageAt', [EXCHANGE, key, pad(toHex(0n))]);
  }
  throw new Error('operator slot not found');
}

/** An EVM transaction from the account, signed through the extension, sent to the fork. */
async function send(to, data) {
  const [nonce, block] = await Promise.all([fork.getTransactionCount({ address: ME }), fork.getBlock()]);
  const unsigned = { type: 'eip1559', chainId: 137, nonce, to, value: 0n, data, maxPriorityFeePerGas: 30_000_000_000n, maxFeePerGas: block.baseFeePerGas * 2n + 30_000_000_000n, gas: 120_000n };
  const sig = await execute({ type: 'evm_tx', tx: { chain_id: 137, nonce, max_priority_fee_per_gas: unsigned.maxPriorityFeePerGas.toString(), max_fee_per_gas: unsigned.maxFeePerGas.toString(), gas_limit: 120000, to, value: '0', data } });
  if (!sig) return false;
  const hash = await fork.request({ method: 'eth_sendRawTransaction', params: [serializeTransaction(unsigned, rsv(sig))] });
  const rc = await fork.waitForTransactionReceipt({ hash });
  console.log(`  fork tx ${hash}: ${rc.status}`);
  return rc.status === 'success';
}

const order = (o) => ({ salt: BigInt(Date.now()), maker: o.maker, signer: o.maker, taker: Z, tokenId: BigInt(o.token), makerAmount: o.pay, takerAmount: o.get, expiration: BigInt(Math.floor(Date.now() / 1000) + 86_400), nonce: 0n, feeRateBps: 0n, side: 0, signatureType: 0 });
const wireOrder = (o) => ({ exchange: EXCHANGE, salt: o.salt.toString(), maker: o.maker, signer: o.signer, taker: o.taker, token_id: o.tokenId.toString(), maker_amount: o.makerAmount.toString(), taker_amount: o.takerAmount.toString(), expiration: Number(o.expiration), nonce: o.nonce.toString(), fee_rate_bps: o.feeRateBps.toString(), side: o.side, signature_type: o.signatureType });

/** The account's order, signed by its EVM key through the extension. */
async function signOrder(o) {
  const sig = await execute({ type: 'polymarket_order', order: wireOrder(o) });
  if (!sig) return null;
  const { r, s, yParity } = rsv(sig);
  const signature = `${r}${s.slice(2)}${(27 + yParity).toString(16)}`;
  const { hashTypedData } = await import('viem');
  const who = await recoverAddress({ hash: hashTypedData({ domain: DOMAIN, types: ORDER_TYPES, primaryType: 'Order', message: o }), signature });
  console.log(`  order signed by ${who} (${who.toLowerCase() === ME.toLowerCase() ? 'the account' : 'NOT the account'})`);
  return { ...o, signature };
}


/** Everything on a fresh fork: funding, the approval, the counterparty, the match. */
async function onFork(yes, no, pay, approveRaw, mine) {
  console.log('fresh Anvil fork of Polygon; funding the account and a counterparty, making an operator');
  const anvil = await startFork();
  try {
    if ((await fork.readContract({ address: EXCHANGE, abi, functionName: 'getComplement', args: [BigInt(yes)] })).toString() !== no) throw new Error('not a YES/NO pair the exchange knows');
    for (const a of [ME, COUNTERPARTY.address, OPERATOR.address]) await rpc('anvil_setBalance', [a, toHex(10n ** 19n)]);
    await fund(USDC_E, ME, pay);
    await fund(USDC_E, COUNTERPARTY.address, pay);
    await makeOperator(OPERATOR.address);
    const bal = async () => Number(await fork.readContract({ address: USDC_E, abi, functionName: 'balanceOf', args: [ME] })) / 1e6;
    console.log(`  the account holds ${await bal()} USDC.e on the fork`);
    const h1 = await fork.request({ method: 'eth_sendRawTransaction', params: [approveRaw] });
    console.log(`  approval on the fork ${h1}: ${(await fork.waitForTransactionReceipt({ hash: h1 })).status}`);
    const cp = createWalletClient({ account: COUNTERPARTY, chain: polygon, transport: http(RPC) });
    await fork.waitForTransactionReceipt({ hash: await cp.writeContract({ address: USDC_E, abi, functionName: 'approve', args: [EXCHANGE, pay] }) });
    const theirs = order({ maker: COUNTERPARTY.address, token: no, pay, get: pay * 2n });
    theirs.signature = await cp.signTypedData({ domain: DOMAIN, types: ORDER_TYPES, primaryType: 'Order', message: theirs });
    console.log('the operator matches them (the exchange mints YES and NO from both payments)');
    const op = createWalletClient({ account: OPERATOR, chain: polygon, transport: http(RPC) });
    await fork.simulateContract({ account: OPERATOR, address: EXCHANGE, abi: [...abi, ...ERRORS], functionName: 'matchOrders', args: [mine, [theirs], pay, [pay]] })
      .catch((e) => console.log(`  simulation: ${e.shortMessage ?? e.message}`.split('\n').slice(0, 3).join(' ')));
    const hash = await op.writeContract({ address: EXCHANGE, abi, functionName: 'matchOrders', args: [mine, [theirs], pay, [pay]], gas: 2_000_000n });
    const rc = await fork.waitForTransactionReceipt({ hash });
    console.log(`  fork tx ${hash}: ${rc.status}`);
    for (const l of rc.logs.filter((l) => l.address.toLowerCase() === USDC_E.toLowerCase() && l.topics[0] === '0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef')) {
      console.log(`  USDC.e ${'0x' + l.topics[1].slice(26)} -> ${'0x' + l.topics[2].slice(26)}: ${Number(BigInt(l.data)) / 1e6}`);
    }
    if (rc.status !== 'success') {
      // Where it failed: the innermost call with an error.
      const t = await rpc('debug_traceTransaction', [hash, { tracer: 'callTracer' }]);
      const walk = (c, d = 0) => { if (c.error) console.log(`  ${'  '.repeat(d)}${c.to} ${c.input?.slice(0, 10)}: ${c.error} ${c.revertReason ?? c.output ?? ''}`); (c.calls ?? []).forEach((x) => walk(x, d + 1)); };
      walk(t);
    }
    const shares = await fork.readContract({ address: CTF, abi, functionName: 'balanceOf', args: [ME, BigInt(yes)] });
    const left = await fork.readContract({ address: USDC_E, abi, functionName: 'balanceOf', args: [ME] });
    console.log(`  the account now holds ${Number(shares) / 1e6} YES shares and ${Number(left) / 1e6} USDC.e`);
  } finally {
    if (!process.env.KEEP_FORK) anvil.kill();
  }
}

const [cmd, a1, a2, a3] = process.argv.slice(2);
const usdc = (x) => BigInt(Math.round(Number(x) * 1e6));
if (cmd === 'settle') {
  const [yes, no] = [a2, a3];
  const pay = usdc(a1);
  // 1. The extension signs: the approval (the account's first Polygon
  //    transaction, nonce 0, generous fees on the fork) and the order.
  console.log(`the account approves the exchange for ${a1} USDC.e (through the extension)`);
  const tx = { type: 'eip1559', chainId: 137, nonce: 0, to: USDC_E, value: 0n, data: encodeFunctionData({ abi, functionName: 'approve', args: [EXCHANGE, pay] }), maxPriorityFeePerGas: 50_000_000_000n, maxFeePerGas: 2_000_000_000_000n, gas: 120_000n };
  const approveSig = await execute({ type: 'evm_tx', tx: { chain_id: 137, nonce: 0, max_priority_fee_per_gas: tx.maxPriorityFeePerGas.toString(), max_fee_per_gas: tx.maxFeePerGas.toString(), gas_limit: 120000, to: USDC_E, value: '0', data: tx.data } });
  if (!approveSig) process.exit(1);
  console.log(`the account buys YES: ${a1} USDC.e for ${Number(pay * 2n) / 1e6} shares (0.50 each)`);
  const mine = await signOrder(order({ maker: ME, token: yes, pay, get: pay * 2n }));
  if (!mine) process.exit(1);
  if (process.env.SAVE_SIGNED) fs.writeFileSync(process.env.SAVE_SIGNED, JSON.stringify({ yes, no, pay: pay.toString(), raw: serializeTransaction(tx, rsv(approveSig)), mine }, (k, v) => (typeof v === 'bigint' ? v.toString() : v)));
  await onFork(yes, no, pay, serializeTransaction(tx, rsv(approveSig)), mine);
} else if (cmd === 'replay') {
  // The fork part again, from what `settle` saved (SAVE_SIGNED): no new signatures.
  const f = JSON.parse(fs.readFileSync(a1, 'utf8'));
  const m = f.mine;
  const mine = { ...m, salt: BigInt(m.salt), tokenId: BigInt(m.tokenId), makerAmount: BigInt(m.makerAmount), takerAmount: BigInt(m.takerAmount), expiration: BigInt(m.expiration), nonce: BigInt(m.nonce), feeRateBps: BigInt(m.feeRateBps) };
  await onFork(f.yes, f.no, BigInt(f.pay), f.raw, mine);
} else if (cmd === 'order-to') {
  console.log(`an order with ${a3} as maker`);
  await signOrder(order({ maker: a3, token: a2, pay: usdc(a1), get: usdc(a1) * 2n }));
} else {
  console.log('usage: see the top of polymarket.mjs');
}
