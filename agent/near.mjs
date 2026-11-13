// Demo agent on NEAR (testnet): calls from the wallet to NEAR protocols. The
// extension checks each call (contract, method, attached NEAR, and for
// token transfers the recipient and message) and the wallet makes it.
//
//   node near.mjs balances                       the wallet's NEAR, wNEAR, USDT, stNEAR, LiNEAR
//   node near.mjs wrap <near>                    NEAR -> wNEAR (registers the wallet first)
//   node near.mjs unwrap <wnear>                 and back
//   node near.mjs ref-swap <wnear> [pool]        swap wNEAR for USDT on Ref (pool 1084; 1% slippage)
//   node near.mjs metapool-stake <near>          stake with Meta Pool for stNEAR
//   node near.mjs metapool-unstake <stnear>      liquid-unstake back to NEAR
//   node near.mjs linear-stake <near>            stake with LiNEAR
//   node near.mjs rnear-stake <near>             stake with Rhea for rNEAR (rnear-sdk's testnet contract)
//   node near.mjs rnear-unstake <near>           delayed unstake of that much NEAR's worth (withdraw_all after ~4 epochs)
//   node near.mjs nearx-stake <near>             stake with Stader for NearX
//   node near.mjs nearx-unstake <near>           delayed unstake of that much NEAR's worth (withdraw after ~4 epochs)
//   node near.mjs nearx-withdraw                 withdraw what has finished unstaking
//   node near.mjs lend-collateral <wnear>        Rhea Lending: put wNEAR in as collateral
//   node near.mjs lend-borrow <amount> [usdc|wnear]  borrow (default wNEAR); it comes to the wallet
//   node near.mjs lend-repay <amount> [usdc|wnear]   repay from the wallet
//   node near.mjs lend-withdraw <wnear>          take collateral back
//   node near.mjs lend-position                  the wallet's collateral and debt
//
// Rhea Lending (formerly Burrow) on testnet: the contract and the message
// shapes rhea-finance/burrow-sdk-python uses; its get_config enables the
// Pyth oracle, so borrows go through execute_with_pyth. It counts amounts
// with extra decimals (USDC 6 + 12), so a USDC borrow is in 18-decimal
// units. On testnet its Pyth feeds are empty and only wNEAR has a (default)
// price, so wNEAR is what can actually be borrowed there.
//
// Env as agent.mjs: MARVII_NEAR_KEYS, RELAYER, WALLET.
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';

const RELAYER = process.env.RELAYER ?? 'http://localhost:8787';
const KEYS = process.env.MARVII_NEAR_KEYS;
const WALLET = process.env.WALLET;
const KEY_ID = process.env.KEY_ID ?? 'aave-agent';
const REF = 'ref-finance-101.testnet';
const WNEAR = 'wrap.testnet';
// Ref pool 1084 on testnet: USDT (usdt.itachicara.testnet) / wNEAR, the
// deepest wNEAR pool there (get_pools).
const POOL = 1084;
const USDT = 'usdt.itachicara.testnet';
const METAPOOL = 'meta-v2.pool.testnet';
const RNEAR = 'lst.ref-dev.testnet'; // rhea-finance/rnear-sdk TESTNET_CONFIG
const LINEAR = 'linear-protocol.testnet';
const NEARX = 'v2-nearx.staderlabs.testnet'; // Stader NearX v2 (get_nearx_price, ft_metadata "NearX")
const YOCTO = 10n ** 24n;
const LENDING = 'contract.1689937928.burrow.testnet';
const LUSDC = 'usdc.fakes.testnet'; // 6 decimals, 12 extra in the lending contract

function agentKey() {
  const { seed_hex } = JSON.parse(fs.readFileSync(path.join(KEYS, 'demo-agent.json'), 'utf8'));
  return crypto.createPrivateKey({ key: Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), Buffer.from(seed_hex, 'hex')]), format: 'der', type: 'pkcs8' });
}
async function relayer(p, body) {
  return (await fetch(RELAYER + p, body ? { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) } : {})).json();
}
const cfg = await relayer('/config');
async function view(account_id, method_name, args = {}) {
  const r = await fetch(cfg.rpc, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'query', params: { request_type: 'call_function', finality: 'final', account_id, method_name, args_base64: Buffer.from(JSON.stringify(args)).toString('base64') } }) }).then((r) => r.json());
  if (r.error || r.result?.error) throw new Error(JSON.stringify(r.error ?? r.result.error));
  return JSON.parse(Buffer.from(r.result.result).toString());
}
const units = (x, d = 24) => {
  const [w, f = ''] = String(x).split('.');
  return (BigInt(w) * 10n ** BigInt(d) + BigInt((f + '0'.repeat(d)).slice(0, d) || '0')).toString();
};
const fmt = (v, d = 24) => (Number(BigInt(v) * 1000000n / 10n ** BigInt(d)) / 1e6).toString();

/** The wallet calls `method` on `receiver_id` with `args`, attaching `deposit` yoctoNEAR. */
async function call(receiver_id, method, args, deposit = '0') {
  const s = await view(cfg.extension, 'get_session', { wallet: WALLET, key_id: KEY_ID });
  const msg = JSON.stringify({ domain: 'marvii-session-v1', extension: cfg.extension, wallet: WALLET, key_id: KEY_ID, nonce: s.nonce + 1, deadline_ms: Date.now() + 300_000,
    action: { type: 'near_call', receiver_id, method, args: JSON.stringify(args), deposit } });
  const out = await relayer('/agent/execute', { msg, signature: crypto.sign(null, crypto.createHash('sha256').update(msg).digest(), agentKey()).toString('hex') });
  if (!out.ok) {
    // The extension refuses with its own short reasons; a contract's panic
    // or a balance error means the call itself failed downstream.
    const r = out.reason ?? out.error ?? '';
    const ours = /not an allowed venue|not allowed|over the daily cap|no cap for|session|token recipient|msg|takes no|venue|grant/i.test(r);
    const downstream = !out.reason || !ours;
    console.log(`  ${downstream ? 'failed' : 'REFUSED by the extension'}: ${r}  (NEAR tx ${out.tx_hash ?? '-'})`);
    return false;
  }
  console.log(`  done: https://testnet.nearblocks.io/txns/${out.tx_hash}`);
  return true;
}
/** Registers the wallet with a token contract if it isn't yet. */
async function register(token, deposit = '0.0125') {
  if (await view(token, 'storage_balance_of', { account_id: WALLET })) return true;
  console.log(`register the wallet on ${token}`);
  return call(token, 'storage_deposit', { account_id: WALLET, registration_only: true }, units(deposit));
}

const [cmd, a1, a2] = process.argv.slice(2);
if (cmd === 'balances') {
  const acct = await fetch(cfg.rpc, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'query', params: { request_type: 'view_account', finality: 'final', account_id: WALLET } }) }).then((r) => r.json());
  console.log(`${WALLET}: ${fmt(acct.result.amount)} NEAR`);
  for (const [t, name, d] of [[WNEAR, 'wNEAR', 24], [USDT, 'USDT', (await view(USDT, 'ft_metadata')).decimals], [METAPOOL, 'stNEAR', 24], [LINEAR, 'LiNEAR', 24], [RNEAR, 'rNEAR', 24], [NEARX, 'NearX', 24]]) {
    console.log(`  ${name}: ${fmt(await view(t, 'ft_balance_of', { account_id: WALLET }), d)}`);
  }
} else if (cmd === 'wrap') {
  if (!(await register(WNEAR))) process.exit(1);
  console.log(`wrap ${a1} NEAR`);
  await call(WNEAR, 'near_deposit', {}, units(a1));
} else if (cmd === 'unwrap') {
  console.log(`unwrap ${a1} wNEAR`);
  await call(WNEAR, 'near_withdraw', { amount: units(a1) }, '1');
} else if (cmd === 'ref-swap') {
  const pool = Number(a2 ?? POOL);
  const amount = units(a1);
  if (!(await register(USDT))) process.exit(1);
  // Another pool is only here to show the refusal: no quote, a nominal minimum.
  const out = pool === POOL ? await view(REF, 'get_return', { pool_id: pool, token_in: WNEAR, amount_in: amount, token_out: USDT }) : '1';
  const min = pool === POOL ? (BigInt(out) * 99n / 100n).toString() : '1';
  console.log(`Ref: swap ${a1} wNEAR for USDT in pool ${pool} (quote ${out}, at least ${min})`);
  const msg = JSON.stringify({ actions: [{ pool_id: pool, token_in: WNEAR, token_out: USDT, amount_in: amount, min_amount_out: min }] });
  await call(WNEAR, 'ft_transfer_call', { receiver_id: REF, amount, msg }, '1');
} else if (cmd === 'metapool-stake') {
  console.log(`Meta Pool: stake ${a1} NEAR`);
  await call(METAPOOL, 'deposit_and_stake', {}, units(a1));
} else if (cmd === 'metapool-unstake') {
  console.log(`Meta Pool: liquid-unstake ${a1} stNEAR`);
  await call(METAPOOL, 'liquid_unstake', { st_near_to_burn: units(a1), min_expected_near: '0' });
} else if (cmd === 'rnear-stake') {
  if (!(await register(RNEAR))) process.exit(1);
  console.log(`Rhea: stake ${a1} NEAR for rNEAR`);
  await call(RNEAR, 'deposit_and_stake', {}, units(a1));
} else if (cmd === 'rnear-unstake') {
  // unstake takes the NEAR value (rnear-sdk converts rNEAR to it first).
  console.log(`Rhea: unstake ${a1} NEAR worth of rNEAR (delayed)`);
  await call(RNEAR, 'unstake', { amount: units(a1) });
} else if (cmd === 'nearx-stake') {
  if (!(await register(NEARX))) process.exit(1);
  console.log(`Stader: stake ${a1} NEAR for NearX`);
  await call(NEARX, 'deposit_and_stake', {}, units(a1));
} else if (cmd === 'nearx-unstake') {
  console.log(`Stader: unstake ${a1} NEAR worth of NearX (delayed)`);
  await call(NEARX, 'unstake', { amount: units(a1) });
} else if (cmd === 'nearx-withdraw') {
  console.log('Stader: withdraw unstaked NEAR');
  await call(NEARX, 'withdraw_all', {});
} else if (cmd === 'linear-stake') {
  console.log(`LiNEAR: stake ${a1} NEAR`);
  await call(LINEAR, 'deposit_and_stake', {}, units(a1));
} else if (cmd === 'lend-collateral') {
  if (!(await register(LENDING, '0.1'))) process.exit(1);
  const amount = units(a1);
  console.log(`Rhea Lending: ${a1} wNEAR as collateral`);
  const msg = JSON.stringify({ Execute: { actions: [{ IncreaseCollateral: { token_id: WNEAR, max_amount: amount } }] } });
  await call(WNEAR, 'ft_transfer_call', { receiver_id: LENDING, amount, msg }, '1');
} else if (cmd === 'lend-borrow' || cmd === 'lend-repay') {
  // [token, its decimals, the lending contract's decimals for it]
  const [token, d, inner] = a2 === 'usdc' ? [LUSDC, 6, 18] : [WNEAR, 24, 24];
  if (!(await register(token))) process.exit(1);
  if (cmd === 'lend-borrow') {
    console.log(`Rhea Lending: borrow ${a1} ${a2 ?? 'wnear'}`);
    await call(LENDING, 'execute_with_pyth', { actions: [{ Borrow: { token_id: token, amount: units(a1, inner) } }, { Withdraw: { token_id: token, max_amount: units(a1, inner) } }] }, '1');
  } else {
    console.log(`Rhea Lending: repay ${a1} ${a2 ?? 'wnear'}`);
    const msg = JSON.stringify({ Execute: { actions: [{ Repay: { token_id: token, max_amount: units(a1, inner) } }] } });
    await call(token, 'ft_transfer_call', { receiver_id: LENDING, amount: units(a1, d), msg }, '1');
  }
} else if (cmd === 'lend-withdraw') {
  const amount = units(a1);
  console.log(`Rhea Lending: withdraw ${a1} wNEAR of collateral`);
  await call(LENDING, 'execute_with_pyth', { actions: [{ DecreaseCollateral: { token_id: WNEAR, amount } }, { Withdraw: { token_id: WNEAR, max_amount: amount } }] }, '1');
} else if (cmd === 'lend-position') {
  const a = await view(LENDING, 'get_account', { account_id: WALLET });
  if (!a) console.log('no Rhea Lending account yet');
  else {
    for (const c of a.collateral ?? []) console.log(`  collateral: ${c.token_id} ${c.balance}`);
    for (const s of a.supplied ?? []) console.log(`  supplied: ${s.token_id} ${s.balance}`);
    for (const b of a.borrowed ?? []) console.log(`  borrowed: ${b.token_id} ${b.balance}`);
  }
} else {
  console.log('usage: see the top of near.mjs');
}
