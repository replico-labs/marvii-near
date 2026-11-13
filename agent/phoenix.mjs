// Demo agent: Phoenix (Solana mainnet's on-chain order book) on a local
// Surfpool fork of mainnet, SOL/USDC market. The extension checks the swap
// (immediate-or-cancel, the account's own token accounts on both sides, a
// budget it can count: quote lots to buy, base lots to sell) and compiles
// the message the MPC network signs.
//
//   surfpool start --network mainnet --no-tui --airdrop <account> --airdrop-amount 5000000000
//   node phoenix.mjs sell <sol>            sell SOL for USDC
//   node phoenix.mjs buy <usdc>            buy SOL with USDC
//   node phoenix.mjs buy-in-base <sol>     a buy budgeted in base lots (refused: no bound on what it spends)
//
// Instructions from @ellipsis-labs/phoenix-sdk 2.0.3; the market's mints,
// vaults and lot sizes from its header on mainnet.
//
// Env as agent.mjs: MARVII_NEAR_KEYS, RELAYER, WALLET; SOL_RPC (default the fork).
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { Connection, PublicKey, SystemProgram } from '@solana/web3.js';
import { createAssociatedTokenAccountIdempotentInstruction, createSyncNativeInstruction, createCloseAccountInstruction } from '@solana/spl-token';
import * as P from '@ellipsis-labs/phoenix-sdk';

const RELAYER = process.env.RELAYER ?? 'http://localhost:8787';
const KEYS = process.env.MARVII_NEAR_KEYS;
const WALLET = process.env.WALLET;
const KEY_ID = process.env.KEY_ID ?? 'aave-agent';
const sol = new Connection(process.env.SOL_RPC ?? 'http://127.0.0.1:8899', 'confirmed');
const PHOENIX = new PublicKey('PhoeNiXZ8ByJGLkxNfZRnkUfjvmuYqLR89jjFHGqdXY');
const MARKET = new PublicKey('4DoNfFBfF7UokCC2FQzriy7yHK6DY6NVdYpuekQ5pRgg'); // SOL/USDC
const TOKEN = new PublicKey('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA');
const ATA_PROGRAM = new PublicKey('ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL');

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
function ata(owner, mint) {
  const [address, bump] = PublicKey.findProgramAddressSync([owner.toBuffer(), TOKEN.toBuffer(), mint.toBuffer()], ATA_PROGRAM);
  return { address, hint: { owner: owner.toBase58(), mint: mint.toBase58(), token_program: TOKEN.toBase58(), bump } };
}
const wire = (ix) => ({
  program: ix.programId.toBase58(),
  accounts: ix.keys.map((k) => ({ pubkey: k.pubkey.toBase58(), signer: k.isSigner, writable: k.isWritable })),
  data: Buffer.from(ix.data).toString('hex'),
});

const core = await loadCore();
const cfg = await relayer('/config');
const me = new PublicKey(core.chain_address('solana', await view(cfg.rpc, cfg.signer, 'derived_public_key', { path: 'solana', predecessor: WALLET, domain_id: 1 })));
const header = P.deserializeMarketData((await sol.getAccountInfo(MARKET)).data).header;
const [base, quote] = [header.baseParams.mintKey, header.quoteParams.mintKey];
const [baseAta, quoteAta] = [ata(me, base), ata(me, quote)];

/** Wrap SOL if selling, the swap, then unwrap what's left of the wrapped SOL. */
function swapIxs(side, baseLots, quoteLots, wrapLamports) {
  const ixs = [
    createAssociatedTokenAccountIdempotentInstruction(me, baseAta.address, me, base),
    createAssociatedTokenAccountIdempotentInstruction(me, quoteAta.address, me, quote),
  ];
  if (wrapLamports) ixs.push(SystemProgram.transfer({ fromPubkey: me, toPubkey: baseAta.address, lamports: wrapLamports }), createSyncNativeInstruction(baseAta.address));
  const orderPacket = P.getImmediateOrCancelOrderPacket({ side, numBaseLots: baseLots, numQuoteLots: quoteLots });
  ixs.push(P.createSwapInstruction({ phoenixProgram: PHOENIX, logAuthority: P.getLogAuthority(), market: MARKET, trader: me, baseAccount: baseAta.address, quoteAccount: quoteAta.address, baseVault: header.baseParams.vaultKey, quoteVault: header.quoteParams.vaultKey }, { orderPacket }));
  ixs.push(createCloseAccountInstruction(baseAta.address, me, me));
  return ixs;
}

async function run(ixs) {
  const { blockhash } = await sol.getLatestBlockhash('confirmed');
  const action = { type: 'sol_tx', recent_blockhash: blockhash, instructions: ixs.map(wire), atas: [baseAta.hint, quoteAta.hint], obligations: [] };
  const s = await view(cfg.rpc, cfg.extension, 'get_session', { wallet: WALLET, key_id: KEY_ID });
  const msg = JSON.stringify({ domain: 'marvii-session-v1', extension: cfg.extension, wallet: WALLET, key_id: KEY_ID, nonce: s.nonce + 1, deadline_ms: Date.now() + 300_000, action });
  const out = await relayer('/agent/execute', { msg, signature: crypto.sign(null, crypto.createHash('sha256').update(msg).digest(), agentKey()).toString('hex') });
  console.log(`  NEAR tx ${out.tx_hash ?? '-'}`);
  if (!out.ok || !out.mpc_signature) {
    console.log(`  REFUSED by the extension: ${out.reason ?? out.error ?? 'no signature'}`);
    return false;
  }
  const message = await view(cfg.rpc, cfg.extension, 'sol_message', { self_address: me.toBase58(), action });
  const raw = core.solana_signed_tx(message, JSON.stringify(out.mpc_signature), me.toBase58());
  const sig = await sol.sendRawTransaction(Buffer.from(raw, 'base64'));
  await sol.confirmTransaction(sig, 'confirmed');
  const st = (await sol.getSignatureStatuses([sig])).value[0];
  console.log(`  fork tx ${sig}: ${st?.err ? 'failed ' + JSON.stringify(st.err) : 'confirmed'}`);
  return !st?.err;
}
async function balances() {
  const usdc = await sol.getTokenAccountBalance(quoteAta.address).then((b) => b.value.uiAmountString, () => '0');
  console.log(`  ${me.toBase58()} on the fork: ${(await sol.getBalance(me)) / 1e9} SOL, ${usdc} USDC`);
}

const baseLot = Number(header.baseLotSize), quoteLot = Number(header.quoteLotSize);
const [cmd, a1] = process.argv.slice(2);
if (cmd === 'sell') {
  const lots = Math.round((Number(a1) * 1e9) / baseLot);
  console.log(`Phoenix: sell ${a1} SOL (${lots} base lots) for USDC`);
  await balances();
  if (await run(swapIxs(P.Side.Ask, lots, 0, lots * baseLot))) await balances();
} else if (cmd === 'buy') {
  const lots = Math.round((Number(a1) * 1e6) / quoteLot);
  console.log(`Phoenix: buy SOL with ${a1} USDC (${lots} quote lots)`);
  await balances();
  if (await run(swapIxs(P.Side.Bid, 0, lots, 0))) await balances();
} else if (cmd === 'buy-in-base') {
  const lots = Math.round((Number(a1) * 1e9) / baseLot);
  console.log(`Phoenix: buy ${a1} SOL budgeted in base lots (no limit on the USDC it takes)`);
  await run(swapIxs(P.Side.Bid, lots, 0, 0));
} else {
  console.log('usage: see the top of phoenix.mjs');
}
