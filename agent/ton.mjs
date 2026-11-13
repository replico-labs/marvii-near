// Demo agent on TON (testnet): TON payments, jetton transfers and EVAA
// lending. The agent sends typed messages; the extension builds the wallet's
// signing message, checks it and has the MPC network sign its hash. The
// agent only adds the signature, wraps it in an external message (with the
// wallet's state init the first time) and sends it.
//
//   node ton.mjs address                      the account, its TON and its jUSDC wallet
//   node ton.mjs pay <ton> <address>          pay a listed address
//   node ton.mjs jetton <amount> <address>    send jUSDC (6 decimals) to a listed address
//   node ton.mjs supply <ton>                 EVAA: supply TON
//   node ton.mjs supply-jusdc <amount>        EVAA: supply (or repay) jUSDC
//   node ton.mjs withdraw <asset> <amount>    EVAA: withdraw what was supplied (asset ton|jusdc; no prices needed without debt)
//   node ton.mjs lst-stake <ton>              Tonstakers: stake TON for tsTON (1 TON fee attached, the rest comes back)
//   node ton.mjs lst-unstake <tston> [now]    burn tsTON; TON comes back at the round's end, or now from the pool's liquidity
//   node ton.mjs lst-balance                  the account's tsTON
//   node ton.mjs nft-send <item> <address>    move one of the account's NFTs to a listed address
//
// Tonstakers' testnet pool from tonstakers-sdk 0.0.20
// (STAKING_CONTRACT_ADDRESS_TESTNET); its tsTON master is the jetton master
// the pool administers (toncenter jetton/masters?admin_address=<pool>).
// Addresses: EVAA's testnet masters and jUSDC from @evaafi/sdk 1.0.3
// (constants/general/testnet, constants/assets/testnet); getUpgradeConfig
// on-chain reports code version 0, the version the SDK targets.
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { Address, beginCell, Cell, storeMessage, storeStateInit } from '@ton/core';
import { WalletContractV4 } from '@ton/ton';

const RELAYER = process.env.RELAYER ?? 'http://localhost:8787';
const KEYS = process.env.MARVII_NEAR_KEYS;
const WALLET = process.env.WALLET;
const KEY_ID = process.env.KEY_ID ?? 'aave-agent';
const TC = process.env.TON_API ?? 'https://testnet.toncenter.com/api/v2';
// EVAA's testnet Pyth master (the classic one refunds every supply, as it has
// everyone's since June).
const EVAA = 'kQBNhBq8D5F1Red9JX64M32PpFrIPRvLvioAgauCDOpfMipw';
const JUSDC = 'kQDaY5yUatYnHei73HBqRX_Ox9LK2XnR7XuCY9MFC2INbfYI';
const TS_POOL = 'kQANFsYyYn-GSZ4oajUJmboDURZU-udMHf9JxzO4vYM_hFP3';
const TSTON = 'kQAiQ2XK7BXePLwemeo-u4wNyjg-wxGeySmaFGEP7R2Mhf4m';
// EVAA asset ids: sha256 of the asset's name (@evaafi/sdk ASSET_ID).
const ASSET = { ton: crypto.createHash('sha256').update('TON').digest('hex'), jusdc: crypto.createHash('sha256').update('jUSDC').digest('hex') };

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
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function tc(p, body) {
  for (let i = 0; ; i++) {
    const r = await (body ? fetch(TC + p, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) }) : fetch(TC + p)).then((r) => r.json());
    if (r.ok || i > 4 || !/limit/i.test(r.error ?? '')) return r;
    await sleep(1500); // about one request a second without a key
  }
}
async function getAddr(contract, method, owner) {
  const r = await tc('/runGetMethod', { address: contract, method, stack: [['tvm.Slice', beginCell().storeAddress(Address.parse(owner)).endCell().toBoc().toString('base64')]] });
  return Cell.fromBase64(r.result.stack[0][1].bytes).beginParse().loadAddress().toString({ testOnly: true });
}

const cfg = await relayer('/config');
const k = await view(cfg.rpc, cfg.signer, 'derived_public_key', { path: 'ton', predecessor: WALLET, domain_id: 1 });
const { default: bs58 } = await import('bs58');
const pk = Buffer.from(bs58.decode(k.slice('ed25519:'.length)));
const wallet = WalletContractV4.create({ workchain: 0, publicKey: pk });
const me = wallet.address.toString({ testOnly: true, bounceable: false });

/** Has the extension build and sign the messages; sends the result. */
async function run(messages) {
  const info = (await tc(`/getWalletInformation?address=${me}`)).result;
  const deployed = info.account_state === 'active';
  const tx = { seqno: deployed ? Number(info.seqno) : 0, valid_until: Math.floor(Date.now() / 1000) + 300, messages };
  const s = await view(cfg.rpc, cfg.extension, 'get_session', { wallet: WALLET, key_id: KEY_ID });
  const msg = JSON.stringify({ domain: 'marvii-session-v1', extension: cfg.extension, wallet: WALLET, key_id: KEY_ID, nonce: s.nonce + 1, deadline_ms: Date.now() + 300_000, action: { type: 'ton_tx', tx } });
  const out = await relayer('/agent/execute', { msg, signature: crypto.sign(null, crypto.createHash('sha256').update(msg).digest(), agentKey()).toString('hex') });
  console.log(`  NEAR tx ${out.tx_hash ?? '-'}`);
  if (!out.ok || !out.mpc_signature) {
    console.log(`  REFUSED by the extension: ${out.reason ?? out.error ?? 'no signature'}`);
    return null;
  }
  // The extension's signing message; the signature must verify over its hash.
  const signing = Cell.fromBoc(Buffer.from(await view(cfg.rpc, cfg.extension, 'ton_unsigned', { wallet: WALLET, key_id: KEY_ID, tx }), 'hex'))[0];
  const sig = Buffer.from(out.mpc_signature.signature);
  const spki = crypto.createPublicKey({ key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), pk]), format: 'der', type: 'spki' });
  if (!crypto.verify(null, signing.hash(), spki, sig)) throw new Error('the signature does not verify for the account');
  const body = beginCell().storeBuffer(sig).storeSlice(signing.beginParse()).endCell();
  const ext = beginCell().store(storeMessage({ info: { type: 'external-in', dest: wallet.address, importFee: 0n }, init: deployed ? undefined : wallet.init, body }, { forceRef: true })).endCell();
  const r = await tc('/sendBoc', { boc: ext.toBoc().toString('base64') });
  if (!r.ok) {
    console.log(`  TON refused: ${r.error}`);
    return null;
  }
  const hash = ext.hash().toString('hex');
  console.log(`  sent (external message ${hash}); waiting for seqno ${tx.seqno + 1}…`);
  for (let i = 0; i < 20; i++) {
    await sleep(3000);
    const n = (await tc(`/getWalletInformation?address=${me}`)).result;
    if (n.account_state === 'active' && Number(n.seqno) > tx.seqno) {
      console.log(`  TON: done  https://testnet.tonviewer.com/transaction/${hash}`);
      return hash;
    }
  }
  console.log('  not confirmed in 60 s');
  return null;
}

const nano = (x) => String(BigInt(Math.round(Number(x) * 1e9)));
const micro = (x) => String(BigInt(Math.round(Number(x) * 1e6)));
const [cmd, a1, a2] = process.argv.slice(2);
if (cmd === 'address') {
  const bal = (await tc(`/getAddressBalance?address=${me}`)).result;
  const jw = await getAddr(JUSDC, 'get_wallet_address', me);
  console.log(`${me}: ${Number(bal) / 1e9} TON\n  jUSDC wallet ${jw}`);
} else if (cmd === 'pay') {
  console.log(`pay ${a1} TON to ${a2}`);
  await run([{ kind: 'pay', to: a2, amount: nano(a1), comment: 'marvii agent' }]);
} else if (cmd === 'jetton') {
  console.log(`send ${a1} jUSDC to ${a2}`);
  await run([{ kind: 'jetton', jetton: JUSDC, to: a2, amount: micro(a1), ton: nano(0.05), forward_ton: '1' }]);
} else if (cmd === 'supply') {
  console.log(`EVAA: supply ${a1} TON`);
  await run([{ kind: 'evaa_supply', master: EVAA, asset: 'ton', amount: nano(a1), ton: nano(0.25) }]);
} else if (cmd === 'supply-jusdc') {
  console.log(`EVAA: supply ${a1} jUSDC`);
  await run([{ kind: 'evaa_supply', master: EVAA, asset: JUSDC, amount: micro(a1), ton: nano(0.35), forward_ton: nano(0.25) }]);
} else if (cmd === 'withdraw') {
  const asset = a1 === 'ton' ? 'ton' : 'jusdc';
  console.log(`EVAA: withdraw ${a2} ${asset}`);
  await run([{ kind: 'evaa_withdraw', master: EVAA, asset_id: ASSET[asset], amount: asset === 'ton' ? nano(a2) : micro(a2), prices: '', ton: nano(0.48) }]);
} else if (cmd === 'lst-stake') {
  console.log(`Tonstakers: stake ${a1} TON for tsTON`);
  await run([{ kind: 'lst_stake', pool: TS_POOL, amount: nano(a1), ton: nano(1) }]);
} else if (cmd === 'lst-unstake') {
  const now = a2 === 'now';
  console.log(`Tonstakers: unstake ${a1} tsTON${now ? ' now (fill or kill)' : ''}`);
  await run([{ kind: 'lst_unstake', pool: TS_POOL, amount: nano(a1), ton: nano(1.05), wait_till_round_end: false, fill_or_kill: now }]);
} else if (cmd === 'nft-send') {
  console.log(`send NFT ${a1} to ${a2}`);
  await run([{ kind: 'nft_transfer', item: a1, to: a2, ton: nano(0.05), forward_ton: '1' }]);
} else if (cmd === 'lst-balance') {
  const jw = await getAddr(TSTON, 'get_wallet_address', me);
  const d = await tc(`/runGetMethod`, { address: jw, method: 'get_wallet_data', stack: [] }).catch(() => null);
  const bal = d?.result?.stack?.[0]?.[1];
  console.log(`tsTON wallet ${jw}: ${bal ? Number(BigInt(bal)) / 1e9 : 0} tsTON`);
} else {
  console.log('usage: see the top of ton.mjs');
}
