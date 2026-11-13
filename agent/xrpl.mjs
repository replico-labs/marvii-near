// Demo agent on the XRP Ledger (testnet): payments, the ledger's order book,
// AMMs and NFTs. The agent sends typed fields; the extension builds the
// transaction, checks it against the grant, and has the MPC network sign it.
//
//   node xrpl.mjs address                         the account's address and balance
//   node xrpl.mjs issuer                          create the demo USD issuer (key kept in MARVII_NEAR_KEYS)
//   node xrpl.mjs pay <drops> <address>           pay XRP to a listed address
//   node xrpl.mjs trust <limit>                   trust line to the demo USD
//   node xrpl.mjs issue <usd>                     the issuer sends the account USD (issuer's own key)
//   node xrpl.mjs offer <usd> <drops>             order book: sell <usd> USD for <drops> XRP
//   node xrpl.mjs offer-eur <drops>               order book with an unlisted currency (refused)
//   node xrpl.mjs amm-create                      the issuer opens an XRP/USD AMM (issuer's own key)
//   node xrpl.mjs amm-deposit <drops>             deposit XRP into the XRP/USD AMM
//   node xrpl.mjs amm-withdraw                    withdraw all of it
//   node xrpl.mjs nft-mint <uri text>             mint an NFT
//   node xrpl.mjs nft-sell <nftoken id> <drops>   offer it for sale
//   node xrpl.mjs nft-offers <nftoken id>         list an NFT's sell offers
//   node xrpl.mjs nft-cancel <offer id>           cancel one of the account's offers
//   node xrpl.mjs issuer-nft                      the issuer mints and lists an NFT (issuer's own key)
//   node xrpl.mjs nft-buy <nftoken id> <drops>    bid for the issuer's NFT; the issuer accepts
//   node xrpl.mjs issuer-ripple                   the issuer enables rippling (needed for the AMM)
//   node xrpl.mjs amm-buy <usd> <drops>           buy USD with XRP; fills against the AMM
//
// Env as agent.mjs: MARVII_NEAR_KEYS, RELAYER, WALLET; XRPL_RPC.
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const xrpl = require('xrpl');
const codec = require('ripple-binary-codec');
const RELAYER = process.env.RELAYER ?? 'http://localhost:8787';
const KEYS = process.env.MARVII_NEAR_KEYS;
const WALLET = process.env.WALLET;
const KEY_ID = process.env.KEY_ID ?? 'aave-agent';
const RPC = process.env.XRPL_RPC ?? 'https://testnet.xrpl-labs.com';
const N = 0xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141n;

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
async function x(method, params = {}) {
  const r = await fetch(RPC, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ method, params: [params] }) }).then((r) => r.json());
  return r.result;
}
async function loadCore() {
  const core = await import('../web/src/core/core.js');
  await core.default({ module_or_path: fs.readFileSync(new URL('../web/src/core/core_bg.wasm', import.meta.url)) });
  return core;
}

/** DER, low S, as the ledger requires. */
function der(rHex, sBig) {
  const enc = (b) => {
    let h = b.toString(16).padStart(64, '0');
    let buf = Buffer.from(h, 'hex');
    while (buf.length > 1 && buf[0] === 0 && !(buf[1] & 0x80)) buf = buf.subarray(1);
    if (buf[0] & 0x80) buf = Buffer.concat([Buffer.from([0]), buf]);
    return Buffer.concat([Buffer.from([0x02, buf.length]), buf]);
  };
  const body = Buffer.concat([enc(BigInt('0x' + rHex)), enc(sBig)]);
  return Buffer.concat([Buffer.from([0x30, body.length]), body]).toString('hex').toUpperCase();
}

async function waitTx(hash) {
  for (let i = 0; i < 30; i++) {
    const t = await x('tx', { transaction: hash });
    if (t?.validated) return t.meta.TransactionResult;
    await new Promise((r) => setTimeout(r, 2000));
  }
  return 'not validated';
}

/** Has the extension build and sign `op`, then submits it. */
async function run(op, from) {
  const cfg = await relayer('/config');
  const info = await x('account_info', { account: from, ledger_index: 'current' });
  const ledger = await x('ledger_current');
  const tx = { sequence: info.account_data.Sequence, fee: '12', last_ledger_sequence: ledger.ledger_current_index + 40, op };
  const s = await view(cfg.rpc, cfg.extension, 'get_session', { wallet: WALLET, key_id: KEY_ID });
  const msg = JSON.stringify({ domain: 'marvii-session-v1', extension: cfg.extension, wallet: WALLET, key_id: KEY_ID, nonce: s.nonce + 1, deadline_ms: Date.now() + 300_000, action: { type: 'xrpl_tx', tx } });
  const out = await relayer('/agent/execute', { msg, signature: crypto.sign(null, crypto.createHash('sha256').update(msg).digest(), agentKey()).toString('hex') });
  console.log(`  NEAR tx ${out.tx_hash ?? '-'}`);
  if (!out.ok || !out.mpc_signature) {
    console.log(`  REFUSED by the extension: ${out.reason ?? out.error ?? 'no signature'}`);
    return null;
  }
  // The extension's own bytes; the ledger checks the signature against them.
  const unsigned = await view(cfg.rpc, cfg.extension, 'xrpl_unsigned', { wallet: WALLET, key_id: KEY_ID, tx });
  const json = codec.decode(unsigned);
  const sig = out.mpc_signature;
  let sv = BigInt('0x' + sig.s.scalar);
  if (sv > N / 2n) sv = N - sv;
  json.TxnSignature = der(sig.big_r.affine_point.slice(2), sv);
  const blob = codec.encode(json);
  const r = await x('submit', { tx_blob: blob });
  console.log(`  submit: ${r.engine_result} ${r.tx_json?.hash}`);
  const result = r.tx_json?.hash ? await waitTx(r.tx_json.hash) : r.engine_result;
  console.log(`  XRPL testnet: ${result}  https://testnet.xrpl.org/transactions/${r.tx_json?.hash}`);
  return result === 'tesSUCCESS' ? r.tx_json.hash : null;
}

/** The demo issuer: a plain XRPL key of our own, standing in for a stablecoin issuer. */
function issuer() {
  const f = path.join(KEYS, 'xrpl-issuer.json');
  if (!fs.existsSync(f)) fs.writeFileSync(f, JSON.stringify({ seed: xrpl.Wallet.generate().seed }));
  return xrpl.Wallet.fromSeed(JSON.parse(fs.readFileSync(f, 'utf8')).seed);
}
async function issuerSubmit(tx) {
  const w = issuer();
  const info = await x('account_info', { account: w.address, ledger_index: 'current' });
  const ledger = await x('ledger_current');
  const t = { Fee: '12', ...tx, Account: w.address, Sequence: info.account_data.Sequence, LastLedgerSequence: ledger.ledger_current_index + 40 };
  const { tx_blob, hash } = w.sign(t);
  const r = await x('submit', { tx_blob });
  console.log(`  issuer ${tx.TransactionType}: ${r.engine_result} → ${await waitTx(hash)}`);
}

const core = await loadCore();
const cfg = await relayer('/config');
const k = await view(cfg.rpc, cfg.signer, 'derived_public_key', { path: 'xrp', predecessor: WALLET, domain_id: 0 });
const from = core.chain_address('xrp', k);
const [cmd, a1, a2] = process.argv.slice(2);
const USD = () => ({ currency: 'USD', issuer: issuer().address });

if (cmd === 'address') {
  const i = await x('account_info', { account: from, ledger_index: 'validated' });
  console.log(`${from}: ${Number(i.account_data?.Balance ?? 0) / 1e6} XRP; issuer ${issuer().address}`);
} else if (cmd === 'issuer') {
  console.log(`issuer ${issuer().address} (fund it with: node xrpl.mjs pay 20000000 ${issuer().address})`);
} else if (cmd === 'issuer-ripple') {
  await issuerSubmit({ TransactionType: 'AccountSet', SetFlag: 8 }); // asfDefaultRipple
} else if (cmd === 'pay') {
  console.log(`pay ${a1} drops to ${a2}`);
  await run({ kind: 'payment', destination: a2, amount: { currency: 'XRP', value: a1 } }, from);
} else if (cmd === 'trust') {
  console.log(`trust line: up to ${a1} USD from ${issuer().address}`);
  await run({ kind: 'trust_set', limit: { ...USD(), value: a1 } }, from);
} else if (cmd === 'issue') {
  await issuerSubmit({ TransactionType: 'Payment', Destination: from, Amount: { ...USD(), value: a1 } });
} else if (cmd === 'offer') {
  console.log(`order book: sell ${a1} USD for ${a2} drops`);
  await run({ kind: 'offer_create', taker_gets: { ...USD(), value: a1 }, taker_pays: { currency: 'XRP', value: a2 } }, from);
} else if (cmd === 'amm-buy') {
  // Buy USD with XRP, immediate-or-cancel: the ledger fills it from the AMM.
  console.log(`buy ${a1} USD for at most ${a2} drops (fills against the XRP/USD AMM)`);
  const h = await run({ kind: 'offer_create', taker_gets: { currency: 'XRP', value: a2 }, taker_pays: { ...USD(), value: a1 }, flags: 0x20000 }, from);
  if (h) {
    const t = await x('tx', { transaction: h });
    const amm = t.meta.AffectedNodes.some((n) => (n.ModifiedNode ?? n.CreatedNode)?.LedgerEntryType === 'AMM' || JSON.stringify(n).includes('AMMID'));
    console.log(`  filled through the AMM: ${amm}`);
  }
} else if (cmd === 'nft-offers') {
  const r = await x('nft_sell_offers', { nft_id: a1 });
  console.log(JSON.stringify(r.offers ?? r.error));
} else if (cmd === 'nft-cancel') {
  console.log(`cancel NFT offer ${a1}`);
  await run({ kind: 'nft_cancel_offers', offers: [a1] }, from);
} else if (cmd === 'issuer-nft') {
  // The issuer mints an NFT and lists it, so the agent has something to buy.
  await issuerSubmit({ TransactionType: 'NFTokenMint', NFTokenTaxon: 0, Flags: 8, URI: Buffer.from('ipfs://marvii-demo-for-sale').toString('hex') });
  const nfts = await x('account_nfts', { account: issuer().address });
  console.log(`  issuer NFTs: ${nfts.account_nfts.map((n) => n.NFTokenID).join(', ')}`);
} else if (cmd === 'nft-buy') {
  // A bid the owner can accept: the price is fixed in the offer the extension builds.
  console.log(`bid ${a2} drops for NFT ${a1} owned by the issuer`);
  const h = await run({ kind: 'nft_buy_offer', nftoken_id: a1, owner: issuer().address, amount: { currency: 'XRP', value: a2 } }, from);
  if (h) {
    const t = await x('tx', { transaction: h });
    const offer = t.meta.offer_id ?? t.meta.AffectedNodes.find((n) => n.CreatedNode?.LedgerEntryType === 'NFTokenOffer')?.CreatedNode.LedgerIndex;
    console.log(`  bid ${offer}; the issuer accepts it:`);
    await issuerSubmit({ TransactionType: 'NFTokenAcceptOffer', NFTokenBuyOffer: offer });
    const mine = await x('account_nfts', { account: from });
    console.log(`  account now holds ${a1}: ${mine.account_nfts.some((n) => n.NFTokenID === a1)}`);
  }
} else if (cmd === 'offer-eur') {
  console.log('order book with EUR, which the grant does not list');
  await run({ kind: 'offer_create', taker_gets: { currency: 'XRP', value: a1 }, taker_pays: { currency: 'EUR', issuer: issuer().address, value: '1' } }, from);
} else if (cmd === 'amm-create') {
  await issuerSubmit({ TransactionType: 'AMMCreate', Fee: '200000', Amount: '5000000', Amount2: { ...USD(), value: '50' }, TradingFee: 500 });
} else if (cmd === 'amm-deposit') {
  console.log(`AMM: deposit ${a1} drops into XRP/USD`);
  await run({ kind: 'amm_deposit', asset: { currency: 'XRP' }, asset2: USD(), amount: { currency: 'XRP', value: a1 } }, from);
} else if (cmd === 'amm-withdraw') {
  console.log('AMM: withdraw everything from XRP/USD');
  await run({ kind: 'amm_withdraw_all', asset: { currency: 'XRP' }, asset2: USD() }, from);
} else if (cmd === 'nft-mint') {
  console.log(`mint an NFT (${a1})`);
  const hash = await run({ kind: 'nft_mint', taxon: 0, uri: Buffer.from(a1).toString('hex') }, from);
  if (hash) {
    const t = await x('tx', { transaction: hash });
    console.log(`  NFT id ${t.meta.nftoken_id}`);
  }
} else if (cmd === 'nft-sell') {
  console.log(`offer NFT ${a1} for ${a2} drops`);
  await run({ kind: 'nft_sell_offer', nftoken_id: a1, amount: { currency: 'XRP', value: a2 } }, from);
} else {
  console.log('usage: see the top of xrpl.mjs');
}
