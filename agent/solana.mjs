// Demo agent on Solana (devnet). It holds only a session key: it sends the
// extension the instructions it wants, the extension checks each one and
// compiles the message itself, the wallet asks the MPC network for an
// ed25519 signature, and the agent broadcasts.
//
//   node solana.mjs address                      the account's Solana address and balance
//   node solana.mjs pay <sol> <address>          pay a listed address
//   node solana.mjs jito-stake <sol>             stake SOL with Jito for JitoSOL
//   node solana.mjs jito-stake-to <sol> <addr>   the same, with JitoSOL to someone else (refused)
//   node solana.mjs jito-unstake <jitosol>       unstake JitoSOL back to SOL
//   node solana.mjs kamino-supply <sol>          lend SOL on Kamino (devnet market)
//   node solana.mjs kamino-redeem <ktokens|all>  take it back
//   node solana.mjs kamino-open                  create the account's Kamino obligation (rent ~0.032 SOL)
//   node solana.mjs kamino-collateral <sol>      deposit SOL as collateral
//   node solana.mjs kamino-borrow <usdc>         borrow devnet USDC against it
//   node solana.mjs kamino-borrow-to <usdc> <a>  the same, paid to someone else (refused)
//   node solana.mjs kamino-repay <usdc|all>      repay
//   node solana.mjs kamino-withdraw <sol|all>    take the collateral back (as SOL)
//   node solana.mjs kamino-position              the obligation's deposits and debt
//   node solana.mjs tensor-buy <mint> [maxSOL]   buy a Metaplex NFT listed on Tensor for SOL
//   node solana.mjs tensor-list <mint> <sol>     list one of the account's NFTs (at or above the passkey's floor)
//   node solana.mjs tensor-delist <mint>         take it down; the NFT comes back
//
// Tensor's instructions come from Tensor's own client, as protean-bot builds
// them (src/solana/integrations/tensor.js): legacy Metaplex NFTs only.
//
// Env as agent.mjs: MARVII_NEAR_KEYS, RELAYER, WALLET; SOL_RPC (default devnet).
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { Connection, PublicKey, SystemProgram, LAMPORTS_PER_SOL, SYSVAR_RENT_PUBKEY } from '@solana/web3.js';
import {
  TOKEN_PROGRAM_ID, ASSOCIATED_TOKEN_PROGRAM_ID, NATIVE_MINT,
  createAssociatedTokenAccountIdempotentInstruction, createSyncNativeInstruction, createCloseAccountInstruction,
} from '@solana/spl-token';
import { StakePoolInstruction, getStakePoolAccount } from '@solana/spl-stake-pool';

const require = createRequire(import.meta.url);
const RELAYER = process.env.RELAYER ?? 'http://localhost:8787';
const KEYS = process.env.MARVII_NEAR_KEYS;
const WALLET = process.env.WALLET;
const KEY_ID = process.env.KEY_ID ?? 'aave-agent';
const sol = new Connection(process.env.SOL_RPC ?? 'https://api.devnet.solana.com', 'confirmed');

// Jito's devnet stake pool (Jito's deployed-programs page; checked on
// devnet: the program owns the pool, the pool mints this JitoSOL).
const JITO = {
  program: new PublicKey('DPoo15wWDqpPJJtS2MUZ49aRxqz5ZaaJCJP4z8bLuib'),
  pool: new PublicKey('JitoY5pcAxWX6iyP2QdFwTznGb8A99PRCUCVVxB46WZ'),
  mint: new PublicKey('J1tos8mqbhdGcF3pgj4PCKyVjzWSURcpLZU7pPGHxSYi'),
};
// Kamino Lend and a devnet market with a wrapped-SOL reserve that is
// refreshed regularly (found on devnet; Kamino publishes no devnet market).
const KAMINO = {
  program: new PublicKey('KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD'),
  market: new PublicKey('Fcv68kert8VhyqjD1Cb7bCtPocHm6sWcrKkpQ2vwnYDM'),
  reserve: new PublicKey('5qTRnm1VKGmNsjfcVB2ShScM2Rez3NjjfrXyh7hSYYiY'),
  // The market's other reserve: Circle's devnet USDC (4zMMC9…).
  usdcReserve: new PublicKey('AgwKsQVbpmkikvnDUU1texZaxVYohm4USzk5g2ZErCup'),
};
const SYSVAR_INSTRUCTIONS = new PublicKey('Sysvar1nstructions1111111111111111111111111');

const keyFile = () => path.join(KEYS, 'demo-agent.json');
function agentKey() {
  const { seed_hex } = JSON.parse(fs.readFileSync(keyFile(), 'utf8'));
  return crypto.createPrivateKey({ key: Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), Buffer.from(seed_hex, 'hex')]), format: 'der', type: 'pkcs8' });
}

async function relayer(p, body) {
  const r = await fetch(RELAYER + p, body ? { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) } : {});
  return r.json();
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

async function me(cfg, core) {
  const k = await view(cfg.rpc, cfg.signer, 'derived_public_key', { path: 'solana', predecessor: WALLET, domain_id: 1 });
  return new PublicKey(core.chain_address('solana', k));
}

/** The token account and its bump, for the extension to derive itself. */
function ata(owner, mint, tokenProgram = TOKEN_PROGRAM_ID) {
  const [address, bump] = PublicKey.findProgramAddressSync([owner.toBuffer(), tokenProgram.toBuffer(), mint.toBuffer()], ASSOCIATED_TOKEN_PROGRAM_ID);
  return { address, hint: { owner: owner.toBase58(), mint: mint.toBase58(), token_program: tokenProgram.toBase58(), bump } };
}

const wire = (ix) => ({
  program: ix.programId.toBase58(),
  accounts: ix.keys.map((k) => ({ pubkey: k.pubkey.toBase58(), signer: k.isSigner, writable: k.isWritable })),
  data: Buffer.from(ix.data).toString('hex'),
});

/** Sends the instructions through the extension; broadcasts if it agrees. */
async function run(ixs, atas, from, core, obligations = []) {
  const cfg = await relayer('/config');
  const { blockhash, lastValidBlockHeight } = await sol.getLatestBlockhash('finalized');
  const action = { type: 'sol_tx', recent_blockhash: blockhash, instructions: ixs.map(wire), atas: atas.map((a) => a.hint), obligations };
  const s = await view(cfg.rpc, cfg.extension, 'get_session', { wallet: WALLET, key_id: KEY_ID });
  const msg = JSON.stringify({ domain: 'marvii-session-v1', extension: cfg.extension, wallet: WALLET, key_id: KEY_ID, nonce: s.nonce + 1, deadline_ms: Date.now() + 300_000, action });
  const out = await relayer('/agent/execute', { msg, signature: crypto.sign(null, crypto.createHash('sha256').update(msg).digest(), agentKey()).toString('hex') });
  console.log(`  NEAR tx ${out.tx_hash ?? '-'}`);
  if (!out.ok || !out.mpc_signature) {
    console.log(`  REFUSED by the extension: ${out.reason ?? out.error ?? 'no signature'}`);
    return false;
  }
  // The message the extension compiled; core checks the signature is the
  // account's over exactly these bytes before building the transaction.
  const message = await view(cfg.rpc, cfg.extension, 'sol_message', { self_address: from.toBase58(), action });
  const raw = core.solana_signed_tx(message, JSON.stringify(out.mpc_signature), from.toBase58());
  const sig = await sol.sendRawTransaction(Buffer.from(raw, 'base64'), { skipPreflight: false });
  // Polled over HTTP (no websocket needed).
  let st;
  for (let i = 0; i < 60; i++) {
    st = (await sol.getSignatureStatuses([sig], { searchTransactionHistory: true })).value[0];
    if (st?.confirmationStatus === 'confirmed' || st?.confirmationStatus === 'finalized' || st?.err) break;
    if ((await sol.getBlockHeight()) > lastValidBlockHeight) break;
    await new Promise((r) => setTimeout(r, 2000));
  }
  const ok = st && !st.err && st.confirmationStatus;
  console.log(`  Solana devnet tx ${sig}: ${ok ? 'confirmed' : 'failed ' + JSON.stringify(st?.err ?? 'not confirmed')}`);
  console.log(`  https://explorer.solana.com/tx/${sig}?cluster=devnet`);
  return !!ok;
}

async function jitoPool() {
  const { account } = await getStakePoolAccount(sol, JITO.pool);
  const p = account.data;
  if (!p.poolMint.equals(JITO.mint)) throw new Error('pool mint changed');
  const [withdrawAuthority] = PublicKey.findProgramAddressSync([JITO.pool.toBuffer(), Buffer.from('withdraw')], JITO.program);
  return { p, withdrawAuthority };
}
const onProgram = (ix, program) => Object.assign(ix, { programId: program });

async function kaminoReserve(address = KAMINO.reserve) {
  const klend = require('@kamino-finance/klend-sdk');
  const info = await sol.getAccountInfo(address);
  if (!info.owner.equals(KAMINO.program)) throw new Error('reserve not owned by Kamino');
  const r = klend.Reserve.decode(info.data);
  if (String(r.lendingMarket) !== KAMINO.market.toBase58()) throw new Error('reserve is in another market');
  const opt = (a) => (a && String(a) !== PublicKey.default.toBase58() ? new PublicKey(String(a)) : KAMINO.program);
  const ti = r.config.tokenInfo;
  return {
    address,
    feeVault: new PublicKey(String(r.liquidity.feeVault)),
    csupply: new PublicKey(String(r.collateral.supplyVault)),
    mint: new PublicKey(String(r.liquidity.mintPubkey)),
    supply: new PublicKey(String(r.liquidity.supplyVault)),
    liqTokenProgram: new PublicKey(String(r.liquidity.tokenProgram)),
    cmint: new PublicKey(String(r.collateral.mintPubkey)),
    oracles: [opt(ti.pythConfiguration.price), opt(ti.switchboardConfiguration.priceAggregator), opt(ti.switchboardConfiguration.twapAggregator), opt(ti.scopeConfiguration.priceFeed)],
    authority: PublicKey.findProgramAddressSync([Buffer.from('lma'), KAMINO.market.toBuffer()], KAMINO.program)[0],
  };
}
const disc = (name) => crypto.createHash('sha256').update(`global:${name}`).digest().subarray(0, 8);
const u64 = (n) => { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(n)); return b; };
const meta = (pubkey, isSigner, isWritable) => ({ pubkey, isSigner, isWritable });

function kaminoRefresh(k) {
  return { programId: KAMINO.program, data: disc('refresh_reserve'), keys: [meta(k.address, false, true), meta(KAMINO.market, false, false), ...k.oracles.map((o) => meta(o, false, false))] };
}

// The account's default obligation (klend-sdk VanillaObligation: tag 0, id
// 0, default seed accounts) and user metadata.
function obligation(owner) {
  const z = PublicKey.default.toBuffer();
  const [address, bump] = PublicKey.findProgramAddressSync([Buffer.from([0]), Buffer.from([0]), owner.toBuffer(), KAMINO.market.toBuffer(), z, z], KAMINO.program);
  return { address, hint: [{ market: KAMINO.market.toBase58(), bump }] };
}
const userMetadata = (owner) => PublicKey.findProgramAddressSync([Buffer.from('user_meta'), owner.toBuffer()], KAMINO.program)[0];

/** The obligation's deposit and borrow reserves, in its own order. */
async function obligationState(o) {
  const klend = require('@kamino-finance/klend-sdk');
  const info = await sol.getAccountInfo(o.address);
  if (!info) return null;
  const ob = klend.Obligation.decode(info.data);
  const live = (x) => String(x) !== PublicKey.default.toBase58();
  return {
    deposits: ob.deposits.filter((d) => live(d.depositReserve)).map((d) => ({ reserve: new PublicKey(String(d.depositReserve)), amount: BigInt(String(d.depositedAmount)) })),
    borrows: ob.borrows.filter((b) => live(b.borrowReserve)).map((b) => ({ reserve: new PublicKey(String(b.borrowReserve)), sf: BigInt(String(b.borrowedAmountSf)) })),
  };
}

/** Refresh every reserve the obligation (and the action) touches, the
 *  action's reserve last, then the obligation: the order klend checks. */
async function kaminoRefreshAll(o, current, extra = []) {
  const st = (await obligationState(o)) ?? { deposits: [], borrows: [] };
  const used = [...st.deposits.map((d) => d.reserve), ...st.borrows.map((b) => b.reserve)];
  const all = [...used, ...extra].filter((r, i, a) => a.findIndex((x) => x.equals(r)) === i);
  const ixs = [];
  for (const r of all.filter((r) => !r.equals(current.address))) ixs.push(kaminoRefresh(await kaminoReserve(r)));
  ixs.push(kaminoRefresh(current));
  ixs.push({ programId: KAMINO.program, data: disc('refresh_obligation'), keys: [meta(KAMINO.market, false, false), meta(o.address, false, true), ...used.map((r) => meta(r, false, false))] });
  return ixs;
}

const core = await loadCore();
const cfg = await relayer('/config');
const from = await me(cfg, core);
const [cmd, a1, a2] = process.argv.slice(2);
const lamports = (x) => BigInt(Math.round(Number(x) * LAMPORTS_PER_SOL));

if (cmd === 'address') {
  console.log(`${from.toBase58()}: ${(await sol.getBalance(from)) / LAMPORTS_PER_SOL} SOL (devnet)`);
} else if (cmd === 'pay') {
  console.log(`pay ${a1} SOL to ${a2}`);
  await run([SystemProgram.transfer({ fromPubkey: from, toPubkey: new PublicKey(a2), lamports: lamports(a1) })], [], from, core);
} else if (cmd === 'jito-stake' || cmd === 'jito-stake-to') {
  const { p, withdrawAuthority } = await jitoPool();
  const to = cmd === 'jito-stake' ? from : new PublicKey(a2);
  const own = ata(to, JITO.mint);
  console.log(`stake ${a1} SOL with Jito, JitoSOL to ${own.address.toBase58()} (owner ${to.toBase58()})`);
  const create = createAssociatedTokenAccountIdempotentInstruction(from, own.address, to, JITO.mint);
  const deposit = onProgram(StakePoolInstruction.depositSol({
    stakePool: JITO.pool, withdrawAuthority, reserveStake: p.reserveStake, fundingAccount: from,
    destinationPoolAccount: own.address, managerFeeAccount: p.managerFeeAccount, referralPoolAccount: own.address,
    poolMint: JITO.mint, lamports: Number(lamports(a1)),
  }), JITO.program);
  await run([create, deposit], [own], from, core);
} else if (cmd === 'jito-unstake') {
  const { p, withdrawAuthority } = await jitoPool();
  const own = ata(from, JITO.mint);
  console.log(`unstake ${a1} JitoSOL for SOL`);
  const withdraw = onProgram(StakePoolInstruction.withdrawSol({
    stakePool: JITO.pool, withdrawAuthority, reserveStake: p.reserveStake, sourcePoolAccount: own.address,
    destinationSystemAccount: from, sourceTransferAuthority: from, managerFeeAccount: p.managerFeeAccount,
    poolMint: JITO.mint, poolTokens: Number(lamports(a1)),
  }), JITO.program);
  await run([withdraw], [own], from, core);
} else if (cmd === 'kamino-supply') {
  const k = await kaminoReserve();
  if (!k.mint.equals(NATIVE_MINT)) throw new Error('reserve is not wrapped SOL');
  const wsol = ata(from, NATIVE_MINT, k.liqTokenProgram);
  const ctok = ata(from, k.cmint);
  const amount = lamports(a1);
  console.log(`lend ${a1} SOL on Kamino (reserve ${KAMINO.reserve.toBase58()})`);
  const deposit = {
    programId: KAMINO.program, data: Buffer.concat([disc('deposit_reserve_liquidity'), u64(amount)]),
    keys: [meta(from, true, false), meta(KAMINO.reserve, false, true), meta(KAMINO.market, false, false), meta(k.authority, false, false),
      meta(k.mint, false, false), meta(k.supply, false, true), meta(k.cmint, false, true), meta(wsol.address, false, true),
      meta(ctok.address, false, true), meta(TOKEN_PROGRAM_ID, false, false), meta(k.liqTokenProgram, false, false), meta(SYSVAR_INSTRUCTIONS, false, false)],
  };
  await run([
    createAssociatedTokenAccountIdempotentInstruction(from, wsol.address, from, NATIVE_MINT, k.liqTokenProgram),
    SystemProgram.transfer({ fromPubkey: from, toPubkey: wsol.address, lamports: amount }),
    createSyncNativeInstruction(wsol.address, k.liqTokenProgram),
    createAssociatedTokenAccountIdempotentInstruction(from, ctok.address, from, k.cmint),
    kaminoRefresh(k),
    deposit,
  ], [wsol, ctok], from, core);
} else if (cmd === 'kamino-redeem') {
  const k = await kaminoReserve();
  const wsol = ata(from, NATIVE_MINT, k.liqTokenProgram);
  const ctok = ata(from, k.cmint);
  const held = BigInt((await sol.getTokenAccountBalance(ctok.address)).value.amount);
  const amount = a1 === 'all' ? held : lamports(a1);
  console.log(`redeem ${amount} Kamino collateral tokens for SOL`);
  const redeem = {
    programId: KAMINO.program, data: Buffer.concat([disc('redeem_reserve_collateral'), u64(amount)]),
    keys: [meta(from, true, false), meta(KAMINO.market, false, false), meta(KAMINO.reserve, false, true), meta(k.authority, false, false),
      meta(k.mint, false, false), meta(k.cmint, false, true), meta(k.supply, false, true), meta(ctok.address, false, true),
      meta(wsol.address, false, true), meta(TOKEN_PROGRAM_ID, false, false), meta(k.liqTokenProgram, false, false), meta(SYSVAR_INSTRUCTIONS, false, false)],
  };
  await run([kaminoRefresh(k), redeem], [wsol, ctok], from, core);
} else if (cmd === 'kamino-open') {
  const o = obligation(from);
  const um = userMetadata(from);
  const ixs = [];
  if (!(await sol.getAccountInfo(um))) {
    ixs.push({ programId: KAMINO.program, data: Buffer.concat([disc('init_user_metadata'), Buffer.alloc(32)]),
      keys: [meta(from, true, false), meta(from, true, true), meta(um, false, true), meta(KAMINO.program, false, false), meta(SYSVAR_RENT_PUBKEY, false, false), meta(SystemProgram.programId, false, false)] });
  }
  if (await sol.getAccountInfo(o.address)) throw new Error(`obligation ${o.address.toBase58()} already exists`);
  ixs.push({ programId: KAMINO.program, data: Buffer.concat([disc('init_obligation'), Buffer.from([0, 0])]),
    keys: [meta(from, true, false), meta(from, true, true), meta(o.address, false, true), meta(KAMINO.market, false, false),
      meta(PublicKey.default, false, false), meta(PublicKey.default, false, false), meta(um, false, false), meta(SYSVAR_RENT_PUBKEY, false, false), meta(SystemProgram.programId, false, false)] });
  console.log(`open the Kamino obligation ${o.address.toBase58()}`);
  await run(ixs, [], from, core, o.hint);
} else if (cmd === 'kamino-collateral') {
  const k = await kaminoReserve();
  const o = obligation(from);
  const wsol = ata(from, NATIVE_MINT, k.liqTokenProgram);
  const amount = lamports(a1);
  console.log(`deposit ${a1} SOL on Kamino as collateral`);
  const deposit = {
    programId: KAMINO.program, data: Buffer.concat([disc('deposit_reserve_liquidity_and_obligation_collateral'), u64(amount)]),
    keys: [meta(from, true, true), meta(o.address, false, true), meta(KAMINO.market, false, false), meta(k.authority, false, false),
      meta(k.address, false, true), meta(k.mint, false, false), meta(k.supply, false, true), meta(k.cmint, false, true),
      meta(k.csupply, false, true), meta(wsol.address, false, true), meta(KAMINO.program, false, false),
      meta(TOKEN_PROGRAM_ID, false, false), meta(k.liqTokenProgram, false, false), meta(SYSVAR_INSTRUCTIONS, false, false)],
  };
  await run([
    createAssociatedTokenAccountIdempotentInstruction(from, wsol.address, from, NATIVE_MINT, k.liqTokenProgram),
    SystemProgram.transfer({ fromPubkey: from, toPubkey: wsol.address, lamports: amount }),
    createSyncNativeInstruction(wsol.address, k.liqTokenProgram),
    ...(await kaminoRefreshAll(o, k)),
    deposit,
  ], [wsol], from, core, o.hint);
} else if (cmd === 'kamino-borrow' || cmd === 'kamino-borrow-to') {
  const k = await kaminoReserve(KAMINO.usdcReserve);
  const o = obligation(from);
  const to = cmd === 'kamino-borrow' ? from : new PublicKey(a2);
  const dest = ata(to, k.mint, k.liqTokenProgram);
  const amount = BigInt(Math.round(Number(a1) * 1e6));
  console.log(`borrow ${a1} USDC on Kamino, to ${dest.address.toBase58()} (owner ${to.toBase58()})`);
  const borrow = {
    programId: KAMINO.program, data: Buffer.concat([disc('borrow_obligation_liquidity'), u64(amount)]),
    keys: [meta(from, true, false), meta(o.address, false, true), meta(KAMINO.market, false, false), meta(k.authority, false, false),
      meta(k.address, false, true), meta(k.mint, false, false), meta(k.supply, false, true), meta(k.feeVault, false, true),
      meta(dest.address, false, true), meta(KAMINO.program, false, false), meta(k.liqTokenProgram, false, false), meta(SYSVAR_INSTRUCTIONS, false, false)],
  };
  await run([
    createAssociatedTokenAccountIdempotentInstruction(from, dest.address, to, k.mint, k.liqTokenProgram),
    ...(await kaminoRefreshAll(o, k, [k.address])),
    borrow,
  ], [dest], from, core, o.hint);
} else if (cmd === 'kamino-repay') {
  const k = await kaminoReserve(KAMINO.usdcReserve);
  const o = obligation(from);
  const src = ata(from, k.mint, k.liqTokenProgram);
  let amt;
  if (a1 === 'all') {
    // The cap counts what is asked for, so ask for what is owed (rounded up), not u64 max.
    const b = (await obligationState(o))?.borrows.find((x) => x.reserve.equals(k.address));
    if (!b) throw new Error('no USDC debt');
    const held = BigInt((await sol.getTokenAccountBalance(src.address)).value.amount);
    // Interest keeps accruing until the transaction lands, so ask for 0.001
    // more than is owed: klend repays at most the debt.
    const owed = (b.sf >> 60n) + 1000n;
    amt = owed < held ? owed : held;
  } else amt = BigInt(Math.round(Number(a1) * 1e6));
  console.log(`repay ${Number(amt) / 1e6} USDC on Kamino`);
  const repay = {
    programId: KAMINO.program, data: Buffer.concat([disc('repay_obligation_liquidity'), u64(amt)]),
    keys: [meta(from, true, false), meta(o.address, false, true), meta(KAMINO.market, false, false), meta(k.address, false, true),
      meta(k.mint, false, false), meta(k.supply, false, true), meta(src.address, false, true), meta(k.liqTokenProgram, false, false), meta(SYSVAR_INSTRUCTIONS, false, false)],
  };
  await run([...(await kaminoRefreshAll(o, k)), repay], [src], from, core, o.hint);
} else if (cmd === 'kamino-withdraw') {
  const k = await kaminoReserve();
  const o = obligation(from);
  const wsol = ata(from, NATIVE_MINT, k.liqTokenProgram);
  const st = await obligationState(o);
  const dep = st?.deposits.find((d) => d.reserve.equals(k.address));
  if (!dep) throw new Error('no SOL collateral');
  // Collateral tokens: all of them, or the SOL amount at the reserve's rate (rounded down).
  let amount = dep.amount;
  if (a1 !== 'all') {
    const klend = require('@kamino-finance/klend-sdk');
    const r = klend.Reserve.decode((await sol.getAccountInfo(k.address)).data);
    const L = r.liquidity, sf = (x) => BigInt(String(x));
    const total = sf(L.totalAvailableAmount) + ((sf(L.borrowedAmountSf) - sf(L.accumulatedProtocolFeesSf) - sf(L.accumulatedReferrerFeesSf) - sf(L.pendingReferrerFeesSf)) >> 60n);
    const supply = BigInt(String(r.collateral.mintTotalSupply));
    amount = (lamports(a1) * supply) / total;
  }
  console.log(`withdraw ${amount} collateral tokens (SOL) from Kamino`);
  const withdraw = {
    programId: KAMINO.program, data: Buffer.concat([disc('withdraw_obligation_collateral_and_redeem_reserve_collateral'), u64(amount)]),
    keys: [meta(from, true, true), meta(o.address, false, true), meta(KAMINO.market, false, false), meta(k.authority, false, false),
      meta(k.address, false, true), meta(k.mint, false, false), meta(k.csupply, false, true), meta(k.cmint, false, true),
      meta(k.supply, false, true), meta(wsol.address, false, true), meta(KAMINO.program, false, false),
      meta(TOKEN_PROGRAM_ID, false, false), meta(k.liqTokenProgram, false, false), meta(SYSVAR_INSTRUCTIONS, false, false)],
  };
  await run([
    createAssociatedTokenAccountIdempotentInstruction(from, wsol.address, from, NATIVE_MINT, k.liqTokenProgram),
    ...(await kaminoRefreshAll(o, k)),
    withdraw,
    createCloseAccountInstruction(wsol.address, from, from, [], k.liqTokenProgram), // unwrap to SOL
  ], [wsol], from, core, o.hint);
} else if (cmd === 'kamino-position') {
  const o = obligation(from);
  const st = await obligationState(o);
  if (!st) console.log(`no obligation yet (${o.address.toBase58()}); run kamino-open`);
  else {
    console.log(`obligation ${o.address.toBase58()}`);
    for (const d of st.deposits) console.log(`  collateral: ${d.amount} tokens of reserve ${d.reserve.toBase58()}`);
    for (const b of st.borrows) console.log(`  debt: ${Number(b.sf) / 2 ** 60 / 1e6} USDC from reserve ${b.reserve.toBase58()}`);
  }
} else if (cmd.startsWith('tensor-')) {
  const tensor = require('@tensor-foundation/marketplace');
  const tmeta = require('@tensor-foundation/mpl-token-metadata');
  const kit = createRequire(require.resolve('@tensor-foundation/marketplace'))('@solana/web3.js');
  const rpc = kit.createSolanaRpc(process.env.SOL_RPC ?? 'https://api.devnet.solana.com');
  const opt = (o) => (o && kit.isSome(o) ? kit.unwrapOption(o) : undefined);
  const fromKit = (ix) => ({ programId: new PublicKey(ix.programAddress), data: Buffer.from(ix.data), keys: ix.accounts.map((a) => meta(new PublicKey(a.address), Boolean(a.role & 2), Boolean(a.role & 1))) });
  const mint = kit.address(a1);
  const md = await tmeta.fetchMaybeMetadata(rpc, (await tmeta.findMetadataPda({ mint }))[0]);
  if (!md.exists) throw new Error('no Metaplex metadata: not an NFT Tensor trades');
  const standard = opt(md.data.tokenStandard);
  if (standard === tmeta.TokenStandard.ProgrammableNonFungible) throw new Error('programmable NFTs are not supported yet');
  const me = kit.createNoopSigner(kit.address(from.toBase58()));
  const own = ata(from, new PublicKey(a1));
  const [listState] = await tensor.findListStatePda({ mint });
  const l = await tensor.fetchMaybeListState(rpc, listState);
  let ix;
  if (cmd === 'tensor-buy') {
    if (!l.exists) throw new Error('not listed on Tensor');
    const d = l.data;
    if (opt(d.currency)) throw new Error('priced in a token, not SOL');
    if (d.cosigner) throw new Error("needs Tensor's cosigner");
    const max = a2 ? lamports(a2) : BigInt(d.amount);
    console.log(`buy ${a1} on Tensor (listed at ${Number(d.amount) / 1e9} SOL, paying at most ${Number(max) / 1e9})`);
    ix = await tensor.getBuyLegacyInstructionAsync({
      payer: me, owner: d.owner, mint, maxAmount: max, rentDestination: d.rentPayer ?? undefined, makerBroker: opt(d.makerBroker),
      takerBroker: me.address, optionalRoyaltyPct: 100, tokenStandard: standard, creators: (opt(md.data.creators) ?? []).map((c) => c.address),
    });
  } else if (cmd === 'tensor-list') {
    console.log(`list ${a1} on Tensor for ${a2} SOL`);
    ix = await tensor.getListLegacyInstructionAsync({ owner: me, payer: me, mint, amount: lamports(a2), makerBroker: me.address, tokenStandard: standard });
  } else if (cmd === 'tensor-delist') {
    if (!l.exists || l.data.owner !== me.address) throw new Error('the account has not listed that NFT');
    console.log(`take ${a1} off Tensor`);
    ix = await tensor.getDelistLegacyInstructionAsync({ owner: me, payer: me, mint, rentDestination: l.data.rentPayer ?? undefined, tokenStandard: standard });
  }
  if (!ix) throw new Error('usage: see the top of solana.mjs');
  await run([fromKit(ix)], [own], from, core);
} else {
  console.log('usage: solana.mjs address | pay <sol> <address> | jito-stake <sol> | jito-stake-to <sol> <address> | jito-unstake <jitosol> | kamino-supply <sol> | kamino-redeem <ktokens|all>');
}
