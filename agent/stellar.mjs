// Demo agent on Stellar (testnet): payments, trust lines, the order book,
// path-payment swaps and AMM pools. The agent sends typed operations; the
// extension builds the transaction, checks it and has the MPC network sign.
//
//   node stellar.mjs address                 the account (funded by friendbot if new) and the demo issuer
//   node stellar.mjs pay <xlm> <address>     pay XLM to a listed address
//   node stellar.mjs trust [usdc]            trust line to the demo USD (or Blend's USDC)
//   node stellar.mjs issue <usd>             the issuer sends the account USD (issuer's own key)
//   node stellar.mjs maker                   the issuer posts USD offers so swaps have liquidity
//   node stellar.mjs offer <usd> <xlm>       order book: sell <usd> USD for <xlm> XLM in total
//   node stellar.mjs swap <xlm> <min usd>    swap XLM for USD (path payment, back to the account)
//   node stellar.mjs pool-deposit <xlm> <usd>  AMM: trust the XLM/USD pool share, deposit
//   node stellar.mjs pool-withdraw           AMM: withdraw all shares
//   node stellar.mjs issuer-pool             the issuer seeds the XLM/USD pool (issuer's own key)
//   node stellar.mjs pool-buy <xlm> <min usd>  swap XLM for USD through the pool
//   node stellar.mjs blend <kind> <xlm|usdc> <amount>  Blend: supply, withdraw, supply_collateral,
//                                            withdraw_collateral, borrow or repay
//   node stellar.mjs blend-position          the account's Blend positions
//
// Env as agent.mjs: MARVII_NEAR_KEYS, RELAYER, WALLET; HORIZON.
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import * as S from '@stellar/stellar-sdk';

const RELAYER = process.env.RELAYER ?? 'http://localhost:8787';
const KEYS = process.env.MARVII_NEAR_KEYS;
const WALLET = process.env.WALLET;
const KEY_ID = process.env.KEY_ID ?? 'aave-agent';
const HORIZON = process.env.HORIZON ?? 'https://horizon-testnet.stellar.org';
const horizon = new S.Horizon.Server(HORIZON);

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
async function loadCore() {
  const core = await import('../web/src/core/core.js');
  await core.default({ module_or_path: fs.readFileSync(new URL('../web/src/core/core_bg.wasm', import.meta.url)) });
  return core;
}
async function friendbot(addr) {
  const r = await fetch(`https://friendbot.stellar.org/?addr=${addr}`);
  console.log(`  friendbot ${addr}: ${r.status}`);
}
async function loadAccount(addr) {
  try { return await horizon.loadAccount(addr); } catch { return null; }
}

/** Has the extension build and sign the operations, then submits. */
async function run(ops, from, soroban) {
  const cfg = await relayer('/config');
  const acct = await horizon.loadAccount(from);
  const tx = { seq_num: (BigInt(acct.sequence) + 1n).toString(), fee: 100 * ops.length, max_time: String(Math.floor(Date.now() / 1000) + 300), ops };
  if (soroban) Object.assign(tx, await soroban(tx));
  const s = await view(cfg.rpc, cfg.extension, 'get_session', { wallet: WALLET, key_id: KEY_ID });
  const msg = JSON.stringify({ domain: 'marvii-session-v1', extension: cfg.extension, wallet: WALLET, key_id: KEY_ID, nonce: s.nonce + 1, deadline_ms: Date.now() + 300_000, action: { type: 'stellar_tx', tx } });
  const out = await relayer('/agent/execute', { msg, signature: crypto.sign(null, crypto.createHash('sha256').update(msg).digest(), agentKey()).toString('hex') });
  console.log(`  NEAR tx ${out.tx_hash ?? '-'}`);
  if (!out.ok || !out.mpc_signature) {
    console.log(`  REFUSED by the extension: ${out.reason ?? out.error ?? 'no signature'}`);
    return null;
  }
  // The extension's own XDR; the signature must verify for the account over its hash.
  const xdrHex = await view(cfg.rpc, cfg.extension, 'stellar_unsigned', { wallet: WALLET, key_id: KEY_ID, tx });
  const kp = S.Keypair.fromPublicKey(from);
  const sig = Buffer.from(out.mpc_signature.signature);
  const env = S.xdr.TransactionEnvelope.envelopeTypeTx(new S.xdr.TransactionV1Envelope({
    tx: S.xdr.Transaction.fromXDR(Buffer.from(xdrHex, 'hex')),
    signatures: [new S.xdr.DecoratedSignature({ hint: kp.signatureHint(), signature: sig })],
  }));
  const t = new S.Transaction(env, S.Networks.TESTNET);
  if (!kp.verify(t.hash(), sig)) throw new Error('signature is not the account’s over this transaction');
  try {
    const r = await horizon.submitTransaction(t);
    console.log(`  Stellar testnet tx ${r.hash}: ${r.successful ? 'success' : 'failed'}  https://stellar.expert/explorer/testnet/tx/${r.hash}`);
    return r.hash;
  } catch (e) {
    console.log(`  Stellar rejected it: ${JSON.stringify(e.response?.data?.extras?.result_codes ?? e.message)}`);
    return null;
  }
}

function issuer() {
  const f = path.join(KEYS, 'stellar-issuer.json');
  if (!fs.existsSync(f)) fs.writeFileSync(f, JSON.stringify({ secret: S.Keypair.random().secret() }));
  return S.Keypair.fromSecret(JSON.parse(fs.readFileSync(f, 'utf8')).secret);
}
async function issuerSubmit(build) {
  const kp = issuer();
  const acct = await horizon.loadAccount(kp.publicKey());
  const b = new S.TransactionBuilder(acct, { fee: '100', networkPassphrase: S.Networks.TESTNET }).setTimeout(120);
  build(b);
  const t = b.build();
  t.sign(kp);
  const r = await horizon.submitTransaction(t);
  console.log(`  issuer tx ${r.hash}: ${r.successful}`);
}

// Blend's testnet pool and tokens: blend-capital/blend-utils
// testnet.contracts.json (TestnetV2, XLM, USDC); the pool's
// get_reserve_list on-chain lists both.
const BLEND_POOL = 'CCEBVDYM32YNYCVNRXQKDFFPISJJCV557CDZEIRBEE4NCV4KHPQ44HGF';
const BLEND_TOKENS = { xlm: 'CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC', usdc: 'CAQCFVLOBK5GIULPNZRGATJJMIZL5BSP7X5YJVMGCPTUEPFM4AVSRCJU' };
// The classic asset behind Blend's testnet USDC token (its contract ID matches).
const BLEND_USDC = { code: 'USDC', issuer: 'GATALTGTWIOT6BUDBCZM3Q4OQ4BO2COLOAZ7IYSKPLC2PMSOPPGF5V56' };
const soroban = new S.rpc.Server(process.env.SOROBAN_RPC ?? 'https://soroban-testnet.stellar.org');

/** Simulates the same Blend submit the extension will build (with the
 *  authorization tree it builds) to get its resources and fee. */
function blendResources(from, requests) {
  return async (tx) => {
    const x = S.xdr;
    const A = new S.Address(from).toScVal();
    const reqs = requests.map((r) => S.nativeToScVal({ address: r.asset, amount: BigInt(r.amount), request_type: ['supply', 'withdraw', 'supply_collateral', 'withdraw_collateral', 'borrow', 'repay'].indexOf(r.kind) },
      { type: { address: ['symbol', 'address'], amount: ['symbol', 'i128'], request_type: ['symbol', 'u32'] } }));
    const args = new x.InvokeContractArgs({ contractAddress: new S.Address(BLEND_POOL).toScAddress(), functionName: 'submit', args: [A, A, A, x.ScVal.scvVec(reqs)] });
    const pulls = {};
    for (const r of requests) if (['supply', 'supply_collateral', 'repay'].includes(r.kind)) pulls[r.asset] = (pulls[r.asset] ?? 0n) + BigInt(r.amount);
    const subs = Object.entries(pulls)
      .sort(([a], [b]) => Buffer.compare(new S.Address(a).toBuffer(), new S.Address(b).toBuffer()))
      .map(([token, amount]) => new x.SorobanAuthorizedInvocation({ subInvocations: [], function: x.SorobanAuthorizedFunction.sorobanAuthorizedFunctionTypeContractFn(
        new x.InvokeContractArgs({ contractAddress: new S.Address(token).toScAddress(), functionName: 'transfer', args: [A, new S.Address(BLEND_POOL).toScVal(), S.nativeToScVal(amount, { type: 'i128' })] })) }));
    const auth = new x.SorobanAuthorizationEntry({ credentials: x.SorobanCredentials.sorobanCredentialsSourceAccount(),
      rootInvocation: new x.SorobanAuthorizedInvocation({ function: x.SorobanAuthorizedFunction.sorobanAuthorizedFunctionTypeContractFn(args), subInvocations: subs }) });
    const op = S.Operation.invokeHostFunction({ func: x.HostFunction.hostFunctionTypeInvokeContract(args), auth: [auth] });
    const draft = new S.TransactionBuilder(new S.Account(from, (BigInt(tx.seq_num) - 1n).toString()), { fee: '100', networkPassphrase: S.Networks.TESTNET, timebounds: { minTime: 0, maxTime: Number(tx.max_time) } }).addOperation(op).build();
    const sim = await soroban.simulateTransaction(draft);
    if (sim.error) throw new Error(`simulation: ${sim.error}`);
    const rf = Math.ceil(Number(sim.minResourceFee) * 1.2);
    const data = sim.transactionData.setResourceFee(rf).build();
    return { soroban_data: data.toXDR('hex'), fee: rf + 10_000 };
  };
}

const core = await loadCore();
const cfg = await relayer('/config');
const k = await view(cfg.rpc, cfg.signer, 'derived_public_key', { path: 'stellar', predecessor: WALLET, domain_id: 1 });
const from = core.chain_address('stellar', k);
const usd = () => ({ code: 'USD', issuer: issuer().publicKey() });
const XLM = { code: 'XLM' };
const stroops = (x) => BigInt(Math.round(Number(x) * 1e7)).toString();
const [cmd, a1, a2] = process.argv.slice(2);

if (cmd === 'address') {
  if (!(await loadAccount(from))) await friendbot(from);
  if (!(await loadAccount(issuer().publicKey()))) await friendbot(issuer().publicKey());
  const a = await loadAccount(from);
  console.log(`${from}: ${a?.balances.map((b) => `${b.balance} ${b.asset_code ?? (b.asset_type === 'native' ? 'XLM' : b.asset_type)}`).join(', ')}; issuer ${issuer().publicKey()}`);
} else if (cmd === 'pay') {
  console.log(`pay ${a1} XLM to ${a2}`);
  await run([{ kind: 'payment', destination: a2, asset: XLM, amount: stroops(a1) }], from);
} else if (cmd === 'trust') {
  // node stellar.mjs trust [usdc]: the demo USD, or Blend's testnet USDC
  const asset = a1 === 'usdc' ? BLEND_USDC : usd();
  console.log(`trust line to ${asset.code}`);
  await run([{ kind: 'change_trust', asset, limit: stroops(10000) }], from);
} else if (cmd === 'issue') {
  await issuerSubmit((b) => b.addOperation(S.Operation.payment({ destination: from, asset: new S.Asset('USD', issuer().publicKey()), amount: a1 })));
} else if (cmd === 'maker') {
  await issuerSubmit((b) => b.addOperation(S.Operation.manageSellOffer({ selling: new S.Asset('USD', issuer().publicKey()), buying: S.Asset.native(), amount: '500', price: '0.5' })));
} else if (cmd === 'offer') {
  // Sell a1 USD for a2 XLM in total: price = XLM per USD.
  console.log(`order book: sell ${a1} USD for ${a2} XLM`);
  const p = Math.round((Number(a2) / Number(a1)) * 1000);
  await run([{ kind: 'sell_offer', selling: usd(), buying: XLM, amount: stroops(a1), price_n: p, price_d: 1000 }], from);
} else if (cmd === 'swap') {
  console.log(`swap ${a1} XLM for at least ${a2} USD`);
  await run([{ kind: 'swap_send', send_asset: XLM, send_amount: stroops(a1), dest_asset: usd(), dest_min: stroops(a2) }], from);
} else if (cmd === 'pool-deposit') {
  console.log(`AMM: deposit ${a1} XLM and ${a2} USD into XLM/USD`);
  await run([
    { kind: 'pool_trust', asset_a: XLM, asset_b: usd() },
    { kind: 'pool_deposit', asset_a: XLM, asset_b: usd(), max_a: stroops(a1), max_b: stroops(a2), min_price: [1, 100], max_price: [100, 1] },
  ], from);
} else if (cmd === 'issuer-pool') {
  // The issuer seeds the XLM/USD pool, at a better price than its own offer.
  const pool = new S.LiquidityPoolAsset(S.Asset.native(), new S.Asset('USD', issuer().publicKey()), 30);
  const pid = S.getLiquidityPoolId('constant_product', pool.getLiquidityPoolParameters()).toString('hex');
  await issuerSubmit((b) => b.addOperation(S.Operation.changeTrust({ asset: pool })).addOperation(S.Operation.liquidityPoolDeposit({ liquidityPoolId: pid, maxAmountA: a1, maxAmountB: a2, minPrice: '0.0001', maxPrice: '10000' })));
} else if (cmd === 'pool-buy') {
  console.log(`swap ${a1} XLM for at least ${a2} USD through the XLM/USD pool`);
  const h = await run([{ kind: 'swap_send', send_asset: XLM, send_amount: stroops(a1), dest_asset: usd(), dest_min: stroops(a2) }], from);
  if (h) {
    const ef = await fetch(`${HORIZON}/transactions/${h}/effects?limit=50`).then((r) => r.json());
    console.log(`  filled through the pool: ${ef._embedded.records.some((e) => e.type === 'liquidity_pool_trade')}`);
  }
} else if (cmd === 'pool-withdraw') {
  const pid = S.getLiquidityPoolId('constant_product', new S.LiquidityPoolAsset(S.Asset.native(), new S.Asset('USD', issuer().publicKey()), 30).getLiquidityPoolParameters()).toString('hex');
  const a = await horizon.loadAccount(from);
  const share = a.balances.find((b) => b.liquidity_pool_id === pid)?.balance ?? '0';
  console.log(`AMM: withdraw ${share} pool shares`);
  await run([{ kind: 'pool_withdraw', asset_a: XLM, asset_b: usd(), amount: stroops(share), min_a: '0', min_b: '0' }], from);
} else if (cmd === 'blend') {
  // node stellar.mjs blend <kind> <xlm|usdc> <amount>
  const [, kind, asset, amount] = process.argv.slice(2);
  const req = [{ kind, asset: BLEND_TOKENS[asset], amount: stroops(amount) }];
  console.log(`Blend: ${kind} ${amount} ${asset.toUpperCase()}`);
  await run([{ kind: 'blend_submit', pool: BLEND_POOL, requests: req }], from, blendResources(from, req));
} else if (cmd === 'blend-position') {
  const draft = new S.TransactionBuilder(new S.Account(from, '0'), { fee: '100', networkPassphrase: S.Networks.TESTNET }).addOperation(new S.Contract(BLEND_POOL).call('get_positions', new S.Address(from).toScVal())).setTimeout(60).build();
  const sim = await soroban.simulateTransaction(draft);
  console.log(JSON.stringify(S.scValToNative(sim.result.retval), (k, v) => (typeof v === 'bigint' ? v.toString() : v)));
} else {
  console.log('usage: see the top of stellar.mjs');
}
