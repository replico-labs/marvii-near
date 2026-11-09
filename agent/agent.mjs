// Demo agent for Marvii on NEAR. It holds only a session key: every request
// is checked by the extension (venue, cap, expiry) before the wallet asks
// the MPC network to sign. The agent then broadcasts the signed transaction.
//
//   node agent.mjs keygen                       creates the agent key, prints its public key
//   node agent.mjs approve <usdc>               approve the Aave pool for <usdc> USDC
//   node agent.mjs supply <usdc>                supply <usdc> USDC to Aave
//   node agent.mjs transfer <usdc> <0x address> try to send USDC away (must be refused)
//   node agent.mjs uniswap <usdc>                swap USDC for WETH on Uniswap V3 (Base Sepolia)
//   node agent.mjs withdraw <usdc>               withdraw USDC from Aave
//   node agent.mjs pay <bitcoin|litecoin|bitcoin-cash|zcash> <amount> <address>
//                                               pay on a payment-only chain
//   node agent.mjs hl-leverage <n>              set ETH leverage on Hyperliquid testnet
//   node agent.mjs hl-order <buy|sell> <size> <price> [Gtc|Ioc|Alo]
//                                               place an ETH order on Hyperliquid testnet
//   node agent.mjs hl-cancel <order id>         cancel it
//
// Env: MARVII_NEAR_KEYS (keeps demo-agent.json; never printed), RELAYER
// (default http://localhost:8787), WALLET (the NEAR wallet account), EVM_RPC
// (default https://sepolia.base.org).
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import {
  createPublicClient, http, keccak256, serializeTransaction, recoverTransactionAddress,
  encodeFunctionData, parseAbi, getAddress, recoverAddress,
} from 'viem';
import { baseSepolia } from 'viem/chains';

const RELAYER = process.env.RELAYER ?? 'http://localhost:8787';
const KEYS = process.env.MARVII_NEAR_KEYS;
const WALLET = process.env.WALLET;
const CHAIN_ID = 84532;
// Aave V3 on Base Sepolia, from bgd-labs/aave-address-book (AaveV3BaseSepolia.sol),
// checked on-chain: the pool's getReservesList() includes this USDC.
const POOL = '0x8bab6d1b75f19e9ed9fce8b9bd338844ff79ae27';
const USDC = '0xba50cd2a20f6da35d788639e581bca8d0b5d4d5f';
// Refusals the delay queue can handle (extension v3, rules.rs).
const NEW_TARGET = ['recipient not allowed', 'token recipient not allowed', 'not an allowed venue', 'spender is not a venue'];
const N = 0xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141n; // secp256k1 order

const evm = createPublicClient({ chain: baseSepolia, transport: http(process.env.EVM_RPC ?? 'https://sepolia.base.org') });
// Uniswap V3 on Base Sepolia: SwapRouter02 and QuoterV2, both checked
// on-chain (factory() is the Uniswap V3 factory, WETH9() is Base's WETH).
const ROUTER = '0x94cc0aac535ccdb3c01d6787d6413c739ae12bc4';
const QUOTER = '0xc5290058841028f1614f3a6f0f5816cad0df5e27';
const WETH = '0x4200000000000000000000000000000000000006';
const abi = parseAbi([
  'function exactInputSingle((address tokenIn, address tokenOut, uint24 fee, address recipient, uint256 amountIn, uint256 amountOutMinimum, uint160 sqrtPriceLimitX96) params) payable returns (uint256)',
  'function quoteExactInputSingle((address tokenIn, address tokenOut, uint256 amountIn, uint24 fee, uint160 sqrtPriceLimitX96) params) returns (uint256 amountOut, uint160, uint32, uint256)',
  'function withdraw(address asset, uint256 amount, address to) returns (uint256)',
  'function approve(address spender, uint256 amount) returns (bool)',
  'function transfer(address to, uint256 amount) returns (bool)',
  'function supply(address asset, uint256 amount, address onBehalfOf, uint16 referralCode)',
]);

const keyFile = () => path.join(KEYS, 'demo-agent.json');
function agentKey() {
  const { seed_hex } = JSON.parse(fs.readFileSync(keyFile(), 'utf8'));
  // PKCS#8 wrapper for a raw ed25519 seed (RFC 8410).
  return crypto.createPrivateKey({ key: Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), Buffer.from(seed_hex, 'hex')]), format: 'der', type: 'pkcs8' });
}
const publicHex = (k) => crypto.createPublicKey(k).export({ format: 'der', type: 'spki' }).subarray(-32).toString('hex');

async function relayer(p, body) {
  const r = await fetch(RELAYER + p, body ? { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) } : {});
  return r.json();
}

async function walletAddress(cfg) {
  const args = Buffer.from(JSON.stringify({ path: 'evm', predecessor: WALLET, domain_id: 0 })).toString('base64');
  const r = await fetch(cfg.rpc, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'query', params: { request_type: 'call_function', finality: 'final', account_id: cfg.signer, method_name: 'derived_public_key', args_base64: args } }) }).then((r) => r.json());
  const k = JSON.parse(Buffer.from(r.result.result).toString());
  // secp256k1:<base58 x||y> -> keccak -> last 20 bytes
  const xy = base58(k.replace('secp256k1:', ''));
  return getAddress('0x' + keccak256(xy).slice(-40));
}

function base58(s) {
  const A = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
  let n = 0n;
  for (const c of s) n = n * 58n + BigInt(A.indexOf(c));
  let hex = n.toString(16);
  if (hex.length % 2) hex = '0' + hex;
  const lead = s.match(/^1*/)[0].length;
  return Uint8Array.from(Buffer.from('00'.repeat(lead) + hex, 'hex'));
}

async function nextSessionNonce(cfg) {
  const args = Buffer.from(JSON.stringify({ wallet: WALLET, key_id: 'aave-agent' })).toString('base64');
  const r = await fetch(cfg.rpc, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'query', params: { request_type: 'call_function', finality: 'final', account_id: cfg.extension, method_name: 'get_session', args_base64: args } }) }).then((r) => r.json());
  const s = JSON.parse(Buffer.from(r.result.result).toString());
  if (!s) throw new Error('no session aave-agent for this wallet');
  return s.nonce + 1;
}

async function send(to, data) {
  const cfg = await relayer('/config');
  const from = await walletAddress(cfg);
  const [nonce, block, prio, gas] = await Promise.all([
    evm.getTransactionCount({ address: from, blockTag: 'pending' }),
    evm.getBlock(),
    evm.estimateMaxPriorityFeePerGas(),
    evm.estimateGas({ account: from, to, data }).catch(() => 200_000n),
  ]);
  const unsigned = {
    type: 'eip1559', chainId: CHAIN_ID, nonce, to, value: 0n, data,
    maxPriorityFeePerGas: prio, maxFeePerGas: block.baseFeePerGas * 2n + prio, gas: (gas * 12n) / 10n,
  };
  const tx = {
    chain_id: CHAIN_ID, nonce, max_priority_fee_per_gas: unsigned.maxPriorityFeePerGas.toString(),
    max_fee_per_gas: unsigned.maxFeePerGas.toString(), gas_limit: Number(unsigned.gas), to, value: '0', data,
  };
  // Each signed message carries the session's next nonce, so it can't replay.
  const signMsg = async () => {
    const msg = JSON.stringify({
      domain: 'marvii-session-v1', extension: cfg.extension, wallet: WALLET, key_id: 'aave-agent',
      nonce: await nextSessionNonce(cfg), deadline_ms: Date.now() + 300_000, action: { type: 'evm_tx', tx },
    });
    return { msg, signature: crypto.sign(null, crypto.createHash('sha256').update(msg).digest(), agentKey()).toString('hex') };
  };
  let out = await relayer('/agent/execute', await signMsg());
  const reason = out.reason ?? out.error;
  console.log(`  NEAR tx ${out.tx_hash ?? '-'}`);
  let mpcSignature = out.mpc_signature;
  if (!out.ok || !mpcSignature) {
    console.log(`  REFUSED by the extension: ${reason ?? 'no signature'}`);
    if (NEW_TARGET.includes(reason)) {
      // A new recipient or venue: propose it. It waits out the extension's
      // delay, during which the owner can cancel it with the passkey.
      const p = await relayer('/agent/propose', await signMsg());
      if (!p.queue_id) { console.log(`  proposal refused: ${p.reason ?? p.error}`); return false; }
      console.log(`  queued as ${p.queue_id}, ready at ${new Date(p.ready_at_ms).toISOString()} (NEAR tx ${p.tx_hash}); the owner can cancel until then`);
      await new Promise((r) => setTimeout(r, Math.max(0, p.ready_at_ms - Date.now()) + 5000));
      out = await relayer(`/queued/${p.queue_id}/execute`, { wallet: WALLET });
      console.log(`  executed after the delay (NEAR tx ${out.tx_hash})`);
      if (!out.ok || !out.mpc_signature) { console.log(`  REFUSED at execution: ${out.reason ?? out.error}`); return false; }
      mpcSignature = out.mpc_signature;
    } else if (!out.pending_id) return false;
  }
  if (!mpcSignature) {
    // Over the cap: the owner can approve this exact transaction with the
    // passkey. Wait for that, then broadcast the same transaction.
    console.log(`  waiting for the owner's approval (request ${out.pending_id})…`);
    const deadline = Date.now() + 10 * 60_000;
    for (;;) {
      const p = await relayer(`/pending/${out.pending_id}`);
      if (p.status === 'approved') { mpcSignature = p.mpc_signature; console.log(`  approved by the passkey (NEAR tx ${p.near_tx})`); break; }
      if (p.status === 'rejected') { console.log('  rejected by the owner'); return false; }
      if (Date.now() > deadline) { console.log('  no approval in 10 minutes'); return false; }
      await new Promise((r) => setTimeout(r, 3000));
    }
  }
  // The signature must be over this exact transaction: recovering the
  // sender below proves it (and the extension's own hash must agree).
  const hash = keccak256(serializeTransaction(unsigned));
  if (out.ok && out.evm_signing_hash && hash.slice(2) !== out.evm_signing_hash) throw new Error(`hash mismatch: ${hash} vs ${out.evm_signing_hash}`);
  const sig = mpcSignature;
  const r = BigInt('0x' + sig.big_r.affine_point.slice(2));
  let s = BigInt('0x' + sig.s.scalar);
  let yParity = sig.recovery_id;
  if (s > N / 2n) { s = N - s; yParity ^= 1; } // low S, as Ethereum requires
  const signed = serializeTransaction(unsigned, { r: '0x' + r.toString(16).padStart(64, '0'), s: '0x' + s.toString(16).padStart(64, '0'), yParity });
  const sender = await recoverTransactionAddress({ serializedTransaction: signed });
  if (sender !== from) throw new Error(`signature recovers to ${sender}, not ${from}`);
  const txHash = await evm.sendRawTransaction({ serializedTransaction: signed });
  const rc = await evm.waitForTransactionReceipt({ hash: txHash });
  console.log(`  Base Sepolia tx ${txHash}: ${rc.status} (block ${rc.blockNumber})`);
  console.log(`  https://sepolia.basescan.org/tx/${txHash}`);
  return rc.status === 'success';
}

// ---- Hyperliquid (testnet) ----
// The agent sends the order's fields; the extension encodes the action and
// builds the EIP-712 digest itself (source "b": Hyperliquid testnet only).
const HL = process.env.HL_API ?? 'https://api.hyperliquid-testnet.xyz';
const HL_ETH = 4; // ETH perp on testnet (`meta` universe index)

async function viewExtension(cfg, method, args) {
  const a = Buffer.from(JSON.stringify(args)).toString('base64');
  const r = await fetch(cfg.rpc, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'query', params: { request_type: 'call_function', finality: 'final', account_id: cfg.extension, method_name: method, args_base64: a } }) }).then((r) => r.json());
  if (r.error || r.result?.error) throw new Error(JSON.stringify(r.error ?? r.result.error));
  return JSON.parse(Buffer.from(r.result.result).toString());
}

/** The JSON action Hyperliquid receives, in the same field order the
 *  extension encodes. */
function hlWire(a) {
  if (a.type === 'hl_order') return { type: 'order', orders: [{ a: a.asset, b: a.is_buy, p: a.price, s: a.size, r: a.reduce_only, t: { limit: { tif: a.tif } } }], grouping: 'na' };
  if (a.type === 'hl_cancel') return { type: 'cancel', cancels: [{ a: a.asset, o: a.oid }] };
  if (a.type === 'hl_leverage') return { type: 'updateLeverage', asset: a.asset, isCross: a.is_cross, leverage: a.leverage };
  throw new Error('not a Hyperliquid action');
}

async function hl(action) {
  const cfg = await relayer('/config');
  const from = await walletAddress(cfg);
  const msg = JSON.stringify({
    domain: 'marvii-session-v1', extension: cfg.extension, wallet: WALLET, key_id: 'aave-agent',
    nonce: await nextSessionNonce(cfg), deadline_ms: Date.now() + 300_000, action,
  });
  const out = await relayer('/agent/execute', { msg, signature: crypto.sign(null, crypto.createHash('sha256').update(msg).digest(), agentKey()).toString('hex') });
  console.log(`  NEAR tx ${out.tx_hash ?? '-'}`);
  if (!out.ok || !out.mpc_signature) {
    console.log(`  REFUSED by the extension: ${out.reason ?? out.error ?? 'no signature'}`);
    return false;
  }
  // The digest the extension had signed, from its own view; the signature
  // must recover to the account's EVM address over exactly that.
  const digest = '0x' + (await viewExtension(cfg, 'hl_signing_hash', { action }));
  const sig = out.mpc_signature;
  const r = BigInt('0x' + sig.big_r.affine_point.slice(2));
  let s = BigInt('0x' + sig.s.scalar);
  let yParity = sig.recovery_id;
  if (s > N / 2n) { s = N - s; yParity ^= 1; }
  const rs = { r: '0x' + r.toString(16).padStart(64, '0'), s: '0x' + s.toString(16).padStart(64, '0'), v: 27 + yParity };
  const signer = await recoverAddress({ hash: digest, signature: { r: rs.r, s: rs.s, yParity } });
  if (signer !== from) throw new Error(`signature recovers to ${signer}, not ${from}`);
  console.log(`  signed by ${from} (Hyperliquid account), digest ${digest.slice(0, 18)}…`);
  const res = await fetch(`${HL}/exchange`, {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ action: hlWire(action), nonce: action.nonce, signature: rs, vaultAddress: null }),
  }).then((r) => r.json());
  console.log(`  Hyperliquid: ${JSON.stringify(res)}`);
  return res.status === 'ok';
}

// ---- Payment-only chains (Bitcoin, Litecoin, Bitcoin Cash, Zcash) ----
// The agent builds the payment with the wallet's own core (the same code
// the page uses), sends the extension its inputs and outputs, and gets one
// signature per input. The extension builds the signing hashes itself.
async function loadCore() {
  const core = await import('../web/src/core/core.js');
  await core.default({ module_or_path: fs.readFileSync(new URL('../web/src/core/core_bg.wasm', import.meta.url)) });
  return core;
}
const UTXO_BROADCAST = { bitcoin: '/utxo/bitcoin/tx', litecoin: '/utxo/litecoin/tx', 'bitcoin-cash': '/chain/bitcoin-cash/tx', zcash: '/chain/zcash/tx' };
const UTXO_COINS = { bitcoin: '/utxo/bitcoin/address/{}/utxo', litecoin: '/utxo/litecoin/address/{}/utxo', 'bitcoin-cash': '/chain/bitcoin-cash/utxos/{}', zcash: '/chain/zcash/utxos/{}' };

async function pay(chain, amount, to) {
  const core = await loadCore();
  const cfg = await relayer('/config');
  const k = await view(cfg.rpc, cfg.signer, 'derived_public_key', { path: chain, predecessor: WALLET, domain_id: 0 });
  const from = core.chain_address(chain, k);
  const units = BigInt(Math.round(Number(amount) * 1e8)).toString();
  const coins = (await relayer(UTXO_COINS[chain].replace('{}', encodeURIComponent(from)))).map((u) => ({ txid: u.txid, vout: u.vout, value: u.value }));
  let plan, branch_id = 0, expiry_height = 0;
  if (chain === 'bitcoin' || chain === 'litecoin') {
    plan = JSON.parse(core.bitcoin_plan(from, to, units, JSON.stringify(coins), 2));
  } else {
    if (chain === 'zcash') {
      const info = await relayer('/chain/zcash/info');
      [branch_id, expiry_height] = [info.branch_id, info.height + 40];
    }
    plan = JSON.parse(core.legacy_plan(chain, from, to, units, JSON.stringify(coins), 2, branch_id, expiry_height));
  }
  if (plan.inputs.length > 2) throw new Error('agents spend at most 2 coins per payment');
  const action = {
    type: 'utxo_pay', chain, branch_id, expiry_height,
    inputs: plan.inputs.map((i) => ({ txid: i.txid, vout: i.vout, value: i.value })),
    outputs: plan.outputs.map((o) => ({ script: core.utxo_script(chain, o.address), value: o.value })),
  };
  console.log(`  ${plan.inputs.length} coin(s) in; out: ${plan.outputs.map((o) => `${o.value} to ${o.address === from ? 'change' : o.address}`).join(', ')}; fee ${plan.fee}`);
  const s = await view(cfg.rpc, cfg.extension, 'get_session', { wallet: WALLET, key_id: 'aave-agent' });
  const msg = JSON.stringify({ domain: 'marvii-session-v1', extension: cfg.extension, wallet: WALLET, key_id: 'aave-agent', nonce: s.nonce + 1, deadline_ms: Date.now() + 300_000, action });
  const out = await relayer('/agent/execute', { msg, signature: crypto.sign(null, crypto.createHash('sha256').update(msg).digest(), agentKey()).toString('hex') });
  console.log(`  NEAR tx ${out.tx_hash ?? '-'}`);
  if (!out.ok || !out.mpc_signatures?.length) {
    console.log(`  REFUSED by the extension: ${out.reason ?? out.error ?? 'no signature'}`);
    return false;
  }
  // core matches each signature to its input and checks it's the account's key.
  const signed = JSON.parse(chain === 'bitcoin' || chain === 'litecoin' ? core.bitcoin_signed_tx(JSON.stringify(plan), JSON.stringify(out.mpc_signatures)) : core.legacy_signed_tx(JSON.stringify(plan), JSON.stringify(out.mpc_signatures)));
  const r = await fetch(RELAYER + UTXO_BROADCAST[chain], { method: 'POST', body: signed.raw }).then((r) => r.json());
  console.log(`  broadcast: ${r.txid ?? r.error}`);
  return !!r.txid;
}

async function view(rpc, account_id, method_name, args) {
  const r = await fetch(rpc, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'query', params: { request_type: 'call_function', finality: 'final', account_id, method_name, args_base64: Buffer.from(JSON.stringify(args)).toString('base64') } }) }).then((r) => r.json());
  if (r.error || r.result?.error) throw new Error(JSON.stringify(r.error ?? r.result.error));
  return JSON.parse(Buffer.from(r.result.result).toString());
}

const usdc = (x) => BigInt(Math.round(Number(x) * 1e6));
const [cmd, a1, a2] = process.argv.slice(2);
if (cmd === 'keygen') {
  if (!fs.existsSync(keyFile())) fs.writeFileSync(keyFile(), JSON.stringify({ seed_hex: crypto.randomBytes(32).toString('hex') }));
  console.log(publicHex(agentKey()));
} else if (cmd === 'approve') {
  console.log(`approve Aave pool for ${a1} USDC`);
  await send(USDC, encodeFunctionData({ abi, functionName: 'approve', args: [POOL, usdc(a1)] }));
} else if (cmd === 'supply') {
  const from = await walletAddress(await relayer('/config'));
  console.log(`supply ${a1} USDC to Aave for ${from}`);
  await send(POOL, encodeFunctionData({ abi, functionName: 'supply', args: [USDC, usdc(a1), from, 0] }));
} else if (cmd === 'transfer') {
  console.log(`transfer ${a1} USDC to ${a2}`);
  await send(USDC, encodeFunctionData({ abi, functionName: 'transfer', args: [a2, usdc(a1)] }));
} else if (cmd === 'uniswap') {
  const from = await walletAddress(await relayer('/config'));
  const amountIn = usdc(a1);
  // The quote, then 2% slippage: the swap reverts if it would give less.
  const { result } = await evm.simulateContract({ address: QUOTER, abi, functionName: 'quoteExactInputSingle', args: [{ tokenIn: USDC, tokenOut: WETH, amountIn, fee: 3000, sqrtPriceLimitX96: 0n }] });
  const minOut = (result[0] * 98n) / 100n;
  console.log(`approve the Uniswap router for ${a1} USDC`);
  if (await send(USDC, encodeFunctionData({ abi, functionName: 'approve', args: [ROUTER, amountIn] }))) {
    console.log(`swap ${a1} USDC for at least ${Number(minOut) / 1e18} WETH, to ${from}`);
    await send(ROUTER, encodeFunctionData({ abi, functionName: 'exactInputSingle', args: [{ tokenIn: USDC, tokenOut: WETH, fee: 3000, recipient: from, amountIn, amountOutMinimum: minOut, sqrtPriceLimitX96: 0n }] }));
  }
} else if (cmd === 'uniswap-to') {
  // An attack: the swap's output to someone else. The extension refuses it.
  const amountIn = usdc(a1);
  console.log(`swap ${a1} USDC with the output to ${a2}`);
  await send(ROUTER, encodeFunctionData({ abi, functionName: 'exactInputSingle', args: [{ tokenIn: USDC, tokenOut: WETH, fee: 3000, recipient: a2, amountIn, amountOutMinimum: 1n, sqrtPriceLimitX96: 0n }] }));
} else if (cmd === 'withdraw') {
  const from = await walletAddress(await relayer('/config'));
  console.log(`withdraw ${a1} USDC from Aave to ${from}`);
  await send(POOL, encodeFunctionData({ abi, functionName: 'withdraw', args: [USDC, usdc(a1), from] }));
} else if (cmd === 'pay') {
  const [chain, amount, to] = process.argv.slice(3);
  console.log(`pay ${amount} on ${chain} to ${to}`);
  await pay(chain, amount, to);
} else if (cmd === 'hl-leverage') {
  console.log(`set ETH leverage to ${a1}x (cross) on Hyperliquid testnet`);
  await hl({ type: 'hl_leverage', asset: HL_ETH, is_cross: true, leverage: Number(a1), nonce: Date.now() });
} else if (cmd === 'hl-order') {
  const [side, size, price, tif = 'Gtc'] = process.argv.slice(3);
  console.log(`${side} ${size} ETH at ${price} (${tif}) on Hyperliquid testnet`);
  await hl({ type: 'hl_order', asset: HL_ETH, is_buy: side === 'buy', price, size, reduce_only: false, tif, nonce: Date.now() });
} else if (cmd === 'hl-cancel') {
  console.log(`cancel ETH order ${a1} on Hyperliquid testnet`);
  await hl({ type: 'hl_cancel', asset: HL_ETH, oid: Number(a1), nonce: Date.now() });
} else {
  console.log('usage: agent.mjs keygen | approve <usdc> | supply <usdc> | transfer <usdc> <0x address> | uniswap <usdc> | uniswap-to <usdc> <0x address> | withdraw <usdc> | hl-leverage <n> | hl-order <buy|sell> <size> <price> [tif] | hl-cancel <oid>');
}
