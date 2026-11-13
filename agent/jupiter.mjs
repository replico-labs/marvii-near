// Demo agent: a Jupiter swap (Solana mainnet's aggregator) against a local
// Surfpool fork of mainnet. Jupiter's API builds the swap for the account
// from live mainnet prices; the extension checks every instruction (the
// account's own token accounts in and out, no platform fee, slippage at most
// 3%) and compiles the message the MPC network signs; the transaction runs
// on the fork, which loads mainnet's programs and pools on demand.
//
//   surfpool start --network mainnet --no-tui --airdrop <account> --airdrop-amount 5000000000
//   node jupiter.mjs swap <sol> [mint]          SOL for USDC (or another mint), on the fork
//   node jupiter.mjs swap-to <sol> <address>    the same with the output to someone else (refused)
//   node jupiter.mjs fee <sol>                  the same with a platform fee (refused)
//   node jupiter.mjs fixture <sol> <file>       write the action, for the extension's tests
//
// Env as agent.mjs: MARVII_NEAR_KEYS, RELAYER, WALLET; SOL_RPC (default the
// fork at http://127.0.0.1:8899); JUP_API (default lite-api.jup.ag/swap/v1).
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { Connection, PublicKey } from '@solana/web3.js';

const RELAYER = process.env.RELAYER ?? 'http://localhost:8787';
const KEYS = process.env.MARVII_NEAR_KEYS;
const WALLET = process.env.WALLET;
const KEY_ID = process.env.KEY_ID ?? 'aave-agent';
const RPC = process.env.SOL_RPC ?? 'http://127.0.0.1:8899';
const JUP = process.env.JUP_API ?? 'https://lite-api.jup.ag/swap/v1';
const sol = new Connection(RPC, 'confirmed');
const WSOL = 'So11111111111111111111111111111111111111112';
const USDC = 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v';
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
  const [address, bump] = PublicKey.findProgramAddressSync([owner.toBuffer(), TOKEN.toBuffer(), new PublicKey(mint).toBuffer()], ATA_PROGRAM);
  return { address, hint: { owner: owner.toBase58(), mint, token_program: TOKEN.toBase58(), bump } };
}
const wire = (ix) => ({
  program: ix.programId,
  accounts: ix.accounts.map((k) => ({ pubkey: k.pubkey, signer: k.isSigner, writable: k.isWritable })),
  data: Buffer.from(ix.data, 'base64').toString('hex'),
});

const core = await loadCore();
const cfg = await relayer('/config');
const me = new PublicKey(core.chain_address('solana', await view(cfg.rpc, cfg.signer, 'derived_public_key', { path: 'solana', predecessor: WALLET, domain_id: 1 })));

/** Jupiter's swap for the account, as the extension's action. */
async function build(amountSol, mint = USDC, { to, fee } = {}) {
  const lamports = Math.round(Number(amountSol) * 1e9);
  // Direct routes in a legacy transaction (no lookup tables: the extension
  // compiles legacy messages only), at most 20 accounts.
  const q = await fetch(`${JUP}/quote?inputMint=${WSOL}&outputMint=${mint}&amount=${lamports}&slippageBps=50&onlyDirectRoutes=true&asLegacyTransaction=true&maxAccounts=20${fee ? '&platformFeeBps=50' : ''}`).then((r) => r.json());
  if (!q.outAmount) throw new Error(`no quote: ${JSON.stringify(q).slice(0, 200)}`);
  const body = { quoteResponse: q, userPublicKey: me.toBase58(), wrapAndUnwrapSol: true, asLegacyTransaction: true, useSharedAccounts: false };
  // A platform fee needs a fee account; any token account of the output mint stands in.
  if (fee) body.feeAccount = ata(new PublicKey('9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM'), mint).address.toBase58();
  const r = await fetch(`${JUP}/swap-instructions`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) }).then((r) => r.json());
  if (r.error) throw new Error(r.error);
  if (r.addressLookupTableAddresses?.length) throw new Error('route needs lookup tables');
  const ixs = [...r.computeBudgetInstructions, ...r.setupInstructions, r.swapInstruction, ...(r.cleanupInstruction ? [r.cleanupInstruction] : [])].map(wire);
  if (to) {
    // Someone else's token account as the destination (account 3 of `route`).
    const swap = ixs.find((i) => i.program === 'JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4');
    swap.accounts[3].pubkey = ata(new PublicKey(to), mint).address.toBase58();
  }
  console.log(`Jupiter: ${amountSol} SOL -> ${Number(q.outAmount) / 1e6} (${q.routePlan.map((p) => p.swapInfo.label).join(' > ')}), at least ${Number(q.otherAmountThreshold) / 1e6}`);
  const { blockhash } = await sol.getLatestBlockhash('confirmed');
  return { type: 'sol_tx', recent_blockhash: blockhash, instructions: ixs, atas: [ata(me, WSOL).hint, ata(me, mint).hint], obligations: [] };
}

async function run(action) {
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
  const sig = await sol.sendRawTransaction(Buffer.from(raw, 'base64'), { skipPreflight: false });
  await sol.confirmTransaction(sig, 'confirmed');
  const st = (await sol.getSignatureStatuses([sig])).value[0];
  console.log(`  fork tx ${sig}: ${st?.err ? 'failed ' + JSON.stringify(st.err) : 'confirmed'}`);
  return !st?.err;
}

async function balances(mint) {
  const lam = await sol.getBalance(me);
  const t = await sol.getTokenAccountBalance(ata(me, mint).address).then((b) => b.value.uiAmountString, () => '0');
  console.log(`  ${me.toBase58()} on the fork: ${lam / 1e9} SOL, ${t} of ${mint.slice(0, 6)}…`);
}

const [cmd, a1, a2] = process.argv.slice(2);
if (cmd === 'swap') {
  const mint = a2 ?? USDC;
  await balances(mint);
  if (await run(await build(a1, mint))) await balances(mint);
} else if (cmd === 'swap-to') {
  await run(await build(a1, USDC, { to: a2 }));
} else if (cmd === 'fee') {
  await run(await build(a1, USDC, { fee: true }));
} else if (cmd === 'fixture') {
  fs.writeFileSync(a2, JSON.stringify({ self_address: me.toBase58(), action: await build(a1) }, null, 1));
  console.log(`wrote ${a2}`);
} else {
  console.log('usage: see the top of jupiter.mjs');
}
