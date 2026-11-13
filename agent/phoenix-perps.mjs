// Demo agent: Phoenix perps (Solana mainnet, Ellipsis Labs' "Rise"
// exchange) on a local Surfpool fork of mainnet, set up like protean-bot's
// Solana trading desk. The account's own key owns the collateral; each
// isolated subaccount is delegated to a trading key of the account's own
// (Chain Signatures, path `phoenix-<sub>`), which Phoenix lets trade the
// subaccount and never withdraw. The extension checks both: what the
// account's key may do (wrap USDC, move collateral between its own Phoenix
// accounts, delegate only to its own trading key) and each order the
// trading key signs (immediate-or-cancel with a limit price, on a market
// the grant lists, up to its size).
//
//   surfpool start --network mainnet --no-tui --airdrop <account> --airdrop-amount 5000000000
//   node phoenix-perps.mjs setup [usdc]          fork only: USDC for the account, SOL for the trading key,
//                                                the account registered on Phoenix (see below)
//   node phoenix-perps.mjs deposit <usdc>        USDC -> Phoenix collateral, into the cross account
//   node phoenix-perps.mjs open <usdc>           open isolated subaccount 1, fund it, delegate it to the trading key
//   node phoenix-perps.mjs trade <long|short> <size> [symbol]    an order by the trading key (limit 2% off the mark)
//   node phoenix-perps.mjs close [symbol]        close subaccount 1's position
//   node phoenix-perps.mjs withdraw              subaccount 1 -> cross -> USDC in the account
//   node phoenix-perps.mjs status
//   refused: too-big, other-market, no-limit, authority-order, delegate-stranger, withdraw-stranger, trader-withdraw
//
// Registration: Phoenix gates new traders; in production its onboarding API
// co-signs (protean-bot's `desk register`, authority not signing). Its API
// isn't reachable from here and its onboarder key isn't ours, so on the fork
// `setup` registers the cross account (a fork keypair pays) and then writes
// the capability flags an onboarded account has (62, read from a live
// mainnet trader) into it with Surfpool's surfnet_setAccount. Every other
// step is a real transaction signed by the MPC network.
//
// Env as agent.mjs: MARVII_NEAR_KEYS, RELAYER, WALLET; SOL_RPC (default the fork);
// TEE_MODULE and KEY_ID to sign as an agent whose key is in OutLayer's TEE.
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { Connection, Keypair, PublicKey, Transaction, TransactionInstruction } from '@solana/web3.js';
import * as rise from '@ellipsis-labs/rise';
import * as L from './phoenix-perps-lib.mjs';

const RELAYER = process.env.RELAYER ?? 'http://localhost:8787';
const KEYS = process.env.MARVII_NEAR_KEYS;
const WALLET = process.env.WALLET;
const KEY_ID = process.env.KEY_ID ?? 'aave-agent';
const RPC = process.env.SOL_RPC ?? 'http://127.0.0.1:8899';
const sol = new Connection(RPC, 'confirmed');
const SUB = 1;
const TOKEN = new PublicKey('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA');
const ATA_PROGRAM = new PublicKey('ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL');
const STRANGER = new PublicKey('9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM');

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
async function rpc(method, params) {
  const r = await fetch(RPC, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }) }).then((r) => r.json());
  if (r.error) throw new Error(`${method}: ${JSON.stringify(r.error)}`);
  return r.result;
}
async function loadCore() {
  const core = await import('../web/src/core/core.js');
  await core.default({ module_or_path: fs.readFileSync(new URL('../web/src/core/core_bg.wasm', import.meta.url)) });
  return core;
}
function ata(owner, mint) {
  const [address, bump] = PublicKey.findProgramAddressSync([owner.toBuffer(), TOKEN.toBuffer(), new PublicKey(mint).toBuffer()], ATA_PROGRAM);
  return { address, hint: { owner: owner.toBase58(), mint: String(mint), token_program: TOKEN.toBase58(), bump } };
}
function trader(owner, sub) {
  const [address, bump] = PublicKey.findProgramAddressSync([Buffer.from('trader'), owner.toBuffer(), Buffer.from([0]), Buffer.from([sub])], new PublicKey(L.PROGRAM));
  return { address, bump, hint: { sub, bump } };
}
/** A kit instruction (the SDK's) in the extension's wire format. */
const wire = (ix) => ({
  program: String(ix.programAddress),
  accounts: ix.accounts.map((a) => ({ pubkey: String(a.address), signer: a.role >= 2, writable: (a.role & 1) === 1 })),
  data: Buffer.from(ix.data).toString('hex'),
});

const core = await loadCore();
const cfg = await relayer('/config');
const derived = async (p) => new PublicKey(core.chain_address('solana', await view(cfg.rpc, cfg.signer, 'derived_public_key', { path: p, predecessor: WALLET, domain_id: 1 })));
const me = await derived('solana');
const tradingKey = await derived(`phoenix-${SUB}`);
const c = L.client(RPC);
const ex = await L.exchange(c);
const cross = trader(me, 0);
const child = trader(me, SUB);
const usdcAta = ata(me, L.USDC);
const phusdAta = ata(me, ex.canonicalMint);

/**
 * TEE_MODULE (a module hash): the agent's key lives in OutLayer's TEE
 * instead (near/sdk OutLayerSigner, the tee-operator account in
 * MARVII_NEAR_KEYS), granted under KEY_ID.
 */
let tee = null;
let paused = false;
async function resume() {
  if (paused) await rpc('surfnet_resumeClock', []);
  paused = false;
}
async function teeAgent() {
  if (!tee) {
    const { OutLayerSigner, MarviiAgent } = await import('../sdk/index.mjs');
    const op = JSON.parse(fs.readFileSync(path.join(KEYS, 'tee-operator.json'), 'utf8'));
    const A = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
    const b58 = (b) => { let n = BigInt('0x' + b.toString('hex')), r = ''; while (n > 0n) { r = A[Number(n % 58n)] + r; n /= 58n; } for (const x of b) { if (x) break; r = '1' + r; } return r; };
    const H = process.env.TEE_MODULE;
    const signer = new OutLayerSigner({ moduleUrl: `https://test.fastfs.io/${op.account_id}/outlayer.testnet/${H}.wasm`, moduleHash: H, operator: { accountId: op.account_id, privateKey: 'ed25519:' + b58(Buffer.concat([Buffer.from(op.seed_hex, 'hex'), Buffer.from(op.public_key_hex, 'hex')])) } });
    tee = new MarviiAgent({ wallet: WALLET, keyId: KEY_ID, signer, relayer: RELAYER });
  }
  return tee;
}

async function execute(action) {
  if (process.env.TEE_MODULE) {
    const out = await (await teeAgent()).execute(action);
    console.log(`  signed in OutLayer's TEE; NEAR tx ${out.tx_hash ?? '-'}`);
    if (!out.ok || !out.mpc_signature) {
      console.log(`  REFUSED by the extension: ${out.reason ?? out.error ?? 'no signature'}`);
      return null;
    }
    return out.mpc_signature;
  }
  const s = await view(cfg.rpc, cfg.extension, 'get_session', { wallet: WALLET, key_id: KEY_ID });
  const msg = JSON.stringify({ domain: 'marvii-session-v1', extension: cfg.extension, wallet: WALLET, key_id: KEY_ID, nonce: s.nonce + 1, deadline_ms: Date.now() + 300_000, action });
  const out = await relayer('/agent/execute', { msg, signature: crypto.sign(null, crypto.createHash('sha256').update(msg).digest(), agentKey()).toString('hex') });
  console.log(`  NEAR tx ${out.tx_hash ?? '-'}`);
  if (!out.ok || !out.mpc_signature) {
    console.log(`  REFUSED by the extension: ${out.reason ?? out.error ?? 'no signature'}`);
    return null;
  }
  return out.mpc_signature;
}
async function send(action, payer) {
  // FIXTURE_DIR: also write the action, for the extension's tests.
  if (process.env.FIXTURE_DIR) fs.writeFileSync(path.join(process.env.FIXTURE_DIR, `perps-${process.argv[2]}.json`), JSON.stringify({ self_address: me.toBase58(), trader_key: tradingKey.toBase58(), action }, null, 1));
  const sig = await execute(action);
  if (!sig) return false;
  const message = await view(cfg.rpc, cfg.extension, 'sol_message', { self_address: payer.toBase58(), action });
  const raw = core.solana_signed_tx(message, JSON.stringify(sig), payer.toBase58());
  // The market's accounts aren't part of what was signed: refresh them again
  // after the signing round-trip (fork only).
  if (action.type === 'phoenix_order') await freshen(lastSymbol, { travel: false });
  try {
    const txid = await sol.sendRawTransaction(Buffer.from(raw, 'base64'));
    await resume();
    await sol.confirmTransaction(txid, 'confirmed');
    const st = (await sol.getSignatureStatuses([txid])).value[0];
    console.log(`  fork tx ${txid}: ${st?.err ? 'failed ' + JSON.stringify(st.err) : 'confirmed'}`);
    return !st?.err;
  } catch (e) {
    console.log(`  the fork rejected it: ${(e.transactionLogs ?? []).slice(process.env.LOGS ? 0 : -3).join(' | ') || e.message}`);
    return false;
  }
}
/** The account's own transaction (its key is the Phoenix authority). */
async function asAccount(ixs, traders = [cross.hint]) {
  const { blockhash } = await sol.getLatestBlockhash('confirmed');
  return send({ type: 'sol_tx', recent_blockhash: blockhash, instructions: ixs.map(wire), atas: [usdcAta.hint, phusdAta.hint, ata(STRANGER, ex.canonicalMint).hint], obligations: [], traders }, me);
}
/** The trading key's orders on subaccount 1, with a 400k compute limit. */
async function asTrader(ixs) {
  const { blockhash } = await sol.getLatestBlockhash('confirmed');
  const limit = { program: 'ComputeBudget111111111111111111111111111111', accounts: [], data: '02' + Buffer.from(new Uint32Array([400_000]).buffer).toString('hex') };
  // Fork only: a TEE agent's signing round-trip (OutLayer, then the MPC
  // network) can outlast a blockhash, so the fork's clock waits for it.
  if (/127\.0\.0\.1|localhost/.test(RPC)) {
    await rpc('surfnet_pauseClock', []);
    paused = true;
  }
  try {
    return await send({ type: 'phoenix_order', sub: SUB, bump: child.bump, recent_blockhash: blockhash, instructions: [limit, ...ixs.map(wire)] }, tradingKey);
  } finally {
    await resume();
  }
}

async function readTrader(t) {
  const info = await sol.getAccountInfo(t.address);
  return info ? rise.decodeTrader(new Uint8Array(info.data)) : null;
}
const usd = (lots) => (Number(lots) / 1e6).toFixed(2);
async function status() {
  const bal = async (a) => sol.getTokenAccountBalance(a).then((b) => b.value.uiAmountString, () => '0');
  console.log(`  account ${me.toBase58()}: ${(await sol.getBalance(me)) / 1e9} SOL, ${await bal(usdcAta.address)} USDC, ${await bal(phusdAta.address)} PhUSD`);
  console.log(`  trading key ${tradingKey.toBase58()}: ${(await sol.getBalance(tradingKey)) / 1e9} SOL`);
  for (const t of [cross, child]) {
    const d = await readTrader(t);
    if (!d) {
      console.log(`  subaccount ${t.hint.sub}: not registered`);
      continue;
    }
    const pos = (d.positions?.entries ?? []).filter((e) => BigInt(e.value.baseLotPosition) !== 0n).map((e) => `asset ${e.key}: ${e.value.baseLotPosition} base lots`).join(', ');
    console.log(`  subaccount ${t.hint.sub} ${t.address.toBase58()}: collateral ${usd(d.state.quoteLotCollateral)} USD, flags ${d.state.flags}, trades for ${d.positionAuthority}${pos ? `, ${pos}` : ''}`);
  }
}

async function market(symbol) {
  const m = await L.market(c, ex, symbol);
  const params = { tickSize: m.tickSize, baseLotsDecimals: m.baseLotsDecimals };
  const mark = rise.ticksToUsdWithMarketParams ? Number(rise.ticksToUsdWithMarketParams(m.oracleTicks, params)) : NaN;
  return { m, params, mark };
}
/**
 * Fork only: the fork loaded mainnet's accounts once, and Phoenix refuses
 * orders against a mark price more than 50 slots old. So before an order,
 * copy the market's live accounts (perp asset map with the oracle prices,
 * order book, spline collection) from mainnet into the fork and move the
 * fork's clock to mainnet's slot (before signing only).
 */
let lastSymbol = 'SOL';
async function freshen(symbol, { travel = true } = {}) {
  lastSymbol = symbol;
  if (!/127\.0\.0\.1|localhost/.test(RPC)) return;
  const main = process.env.MAINNET_RPC ?? 'https://api.mainnet-beta.solana.com';
  const m = await L.market(c, ex, symbol);
  const call = (method, params) => fetch(main, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }) }).then((r) => r.json()).then((r) => r.result);
  for (const a of [ex.perpAssetMap, m.marketAddress, m.splineCollection]) {
    const v = (await call('getAccountInfo', [String(a), { encoding: 'base64' }])).value;
    await rpc('surfnet_setAccount', [String(a), { data: Buffer.from(v.data[0], 'base64').toString('hex'), lamports: v.lamports, owner: v.owner, executable: false }]);
  }
  const slot = await call('getSlot', []);
  // Moving the clock after signing would expire the signed blockhash.
  if (travel && slot > (await sol.getSlot())) await rpc('surfnet_timeTravel', [{ absoluteSlot: slot }]);
}

async function orderIx(symbol, side, size, { limit = true, minFill } = {}) {
  await freshen(symbol);
  const { m, params, mark } = await market(symbol);
  const price = side === rise.Side.Bid ? mark * 1.02 : mark * 0.98;
  console.log(`  ${symbol}: mark ${mark.toFixed(4)} USD, ${side === rise.Side.Bid ? 'buy' : 'sell'} ${size} at ${limit ? price.toFixed(4) : 'any price'}`);
  return rise.buildPlaceMarketOrderIxResolved({
    exchange: ex,
    market: { marketAddress: m.marketAddress, splineCollection: m.splineCollection },
    trader: { authority: me.toBase58(), positionAuthority: tradingKey.toBase58(), traderAccount: child.address.toBase58() },
    orderPacket: rise.buildMarketOrderPacketFromMarketParams({ side, priceLimitUsd: limit ? price.toFixed(6) : null, baseUnits: String(size), minBaseUnitsToFill: minFill ?? String(size) }, params),
  });
}
async function heldPosition() {
  const d = await readTrader(child);
  const [entry] = (d?.positions?.entries ?? []).filter((e) => BigInt(e.value.baseLotPosition) !== 0n);
  return entry ? BigInt(entry.value.baseLotPosition) : 0n;
}

const [cmd, a1, a2, a3] = process.argv.slice(2);
if (cmd === 'setup') {
  console.log('Fork setup (not through the extension; see the header)');
  await rpc('surfnet_setTokenAccount', [me.toBase58(), L.USDC, { amount: Math.round(Number(a1 ?? 100) * 1e6) }, TOKEN.toBase58()]);
  await rpc('requestAirdrop', [tradingKey.toBase58(), 100_000_000]);
  if (!(await readTrader(cross))) {
    const payer = Keypair.generate();
    await sol.confirmTransaction(await sol.requestAirdrop(payer.publicKey, 1e9), 'confirmed');
    const ix = rise.buildRegisterTraderIxResolved({ exchange: ex, trader: { payer: payer.publicKey.toBase58(), authority: me.toBase58(), traderAccount: cross.address.toBase58() }, maxPositions: 128n, traderPdaIndex: 0, traderSubaccountIndex: 0 });
    const tx = new Transaction().add(new TransactionInstruction({ programId: new PublicKey(ix.programAddress), keys: ix.accounts.map((a) => ({ pubkey: new PublicKey(a.address), isSigner: a.role >= 2, isWritable: (a.role & 1) === 1 })), data: Buffer.from(ix.data) }));
    console.log(`  registered the cross account (a fork keypair paid, the account didn't sign): ${await sol.sendTransaction(tx, [payer])}`);
  }
  // Onboarding's capability flags, as on a live mainnet trader: state.flags
  // is the u32 after the 32-byte authority and the 8-byte collateral.
  const info = await sol.getAccountInfo(cross.address);
  const data = Buffer.from(info.data);
  const at = data.indexOf(me.toBuffer()) + 32 + 8;
  data.writeUInt32LE(62, at);
  if (rise.decodeTrader(new Uint8Array(data)).state.flags !== 62) throw new Error('flags not where expected');
  await rpc('surfnet_setAccount', [cross.address.toBase58(), { data: data.toString('hex'), lamports: info.lamports, owner: L.PROGRAM, executable: false }]);
  console.log('  cross account activated (flags 62, standing in for Phoenix\'s onboarder)');
  await status();
} else if (cmd === 'deposit') {
  const amount = BigInt(Math.round(Number(a1) * 1e6));
  console.log(`Deposit ${a1} USDC into Phoenix (Ember wraps it, then the cross account)`);
  const b = rise.buildDepositIxsResolved({ exchange: ex, feePayer: me.toBase58(), trader: { authority: me.toBase58(), traderAccount: cross.address.toBase58(), usdcTokenAccount: usdcAta.address.toBase58(), phoenixTokenAccount: phusdAta.address.toBase58() }, amount });
  if (await asAccount(L.listOf(b))) await status();
} else if (cmd === 'open') {
  const amount = BigInt(Math.round(Number(a1) * 1e6));
  console.log(`Open isolated subaccount ${SUB}, move ${a1} USD of collateral into it, delegate it to the trading key`);
  const ixs = [];
  if (!(await readTrader(child))) {
    ixs.push(
      rise.buildRegisterTraderIxResolved({ exchange: ex, trader: { payer: me.toBase58(), authority: me.toBase58(), traderAccount: child.address.toBase58() }, maxPositions: 1n, traderPdaIndex: 0, traderSubaccountIndex: SUB }),
      rise.buildSyncParentToChildIxResolved({ exchange: ex, trader: { authority: me.toBase58(), parentTraderAccount: cross.address.toBase58(), childTraderAccount: child.address.toBase58() } }),
    );
  }
  ixs.push(
    rise.buildTransferCollateralIxResolved({ exchange: ex, trader: { authority: me.toBase58(), srcTraderAccount: cross.address.toBase58(), dstTraderAccount: child.address.toBase58() }, amount }),
    rise.buildDelegateTraderIxResolved({ exchange: ex, trader: { authority: me.toBase58(), traderAccount: child.address.toBase58() }, newPositionAuthority: tradingKey.toBase58() }),
  );
  const before = await sol.getBalance(me);
  if (await asAccount(ixs, [cross.hint, child.hint])) {
    console.log(`  the account paid ${(before - (await sol.getBalance(me))) / 1e9} SOL (fee and the subaccount's rent)`);
    await status();
  }
} else if (cmd === 'trade') {
  const side = a1 === 'short' ? rise.Side.Ask : rise.Side.Bid;
  console.log(`The trading key opens a ${a1} of ${a2} ${a3 ?? 'SOL'} on subaccount ${SUB}`);
  if (await asTrader([await orderIx(a3 ?? 'SOL', side, a2)])) await status();
} else if (cmd === 'close') {
  const lots = await heldPosition();
  if (!lots) {
    console.log('  no position');
  } else {
    const { params } = await market(a1 ?? 'SOL');
    const size = (Number(lots < 0n ? -lots : lots) / 10 ** params.baseLotsDecimals).toString();
    console.log(`The trading key closes ${lots} base lots`);
    if (await asTrader([await orderIx(a1 ?? 'SOL', lots > 0n ? rise.Side.Ask : rise.Side.Bid, size)])) await status();
  }
} else if (cmd === 'withdraw') {
  const d = await readTrader(child);
  const crossD = await readTrader(cross);
  const amount = BigInt(d.state.quoteLotCollateral) + BigInt(crossD.state.quoteLotCollateral);
  console.log(`Withdraw ${usd(amount)} USD: subaccount ${SUB} back to the cross account, out of Phoenix, unwrapped to USDC`);
  const back = rise.buildTransferCollateralChildToParentIxResolved({ exchange: ex, trader: { authority: me.toBase58(), childTraderAccount: child.address.toBase58(), parentTraderAccount: cross.address.toBase58() } });
  const out = rise.buildWithdrawIxsResolved({ exchange: ex, feePayer: me.toBase58(), trader: { authority: me.toBase58(), traderAccount: cross.address.toBase58(), usdcTokenAccount: usdcAta.address.toBase58(), phoenixTokenAccount: phusdAta.address.toBase58() }, amount });
  if (await asAccount([back, ...L.listOf(out)], [cross.hint, child.hint])) await status();
} else if (cmd === 'status') {
  await status();
} else if (cmd === 'too-big') {
  console.log('The trading key: an order over the grant\'s size per order (refused)');
  await asTrader([await orderIx('SOL', rise.Side.Bid, a1 ?? '20')]);
} else if (cmd === 'other-market') {
  console.log('The trading key: an order on a market the grant doesn\'t list (refused)');
  await asTrader([await orderIx(a1 ?? 'BTC', rise.Side.Bid, '0.0001')]);
} else if (cmd === 'no-limit') {
  console.log('The trading key: an order with no limit price (refused)');
  await asTrader([await orderIx('SOL', rise.Side.Bid, '0.1', { limit: false })]);
} else if (cmd === 'authority-order') {
  console.log('The account\'s own key placing an order on its cross account (refused: orders go through the trading key)');
  const { m, params, mark } = await market('SOL');
  const ix = rise.buildPlaceMarketOrderIxResolved({ exchange: ex, market: { marketAddress: m.marketAddress, splineCollection: m.splineCollection }, trader: { authority: me.toBase58(), traderAccount: cross.address.toBase58() }, orderPacket: rise.buildMarketOrderPacketFromMarketParams({ side: rise.Side.Bid, priceLimitUsd: (mark * 1.02).toFixed(4), baseUnits: '0.1' }, params) });
  await asAccount([ix]);
} else if (cmd === 'delegate-stranger') {
  console.log('The account\'s key delegating subaccount 1 to someone else\'s key (refused)');
  await asAccount([rise.buildDelegateTraderIxResolved({ exchange: ex, trader: { authority: me.toBase58(), traderAccount: child.address.toBase58() }, newPositionAuthority: STRANGER.toBase58() })], [cross.hint, child.hint]);
} else if (cmd === 'withdraw-stranger') {
  console.log('Withdrawing from Phoenix into someone else\'s collateral account (refused)');
  const ix = rise.buildWithdrawFundsIxResolved({ exchange: ex, trader: { authority: me.toBase58(), traderAccount: cross.address.toBase58(), destinationTokenAccount: ata(STRANGER, ex.canonicalMint).address.toBase58() }, amount: 1_000_000n });
  await asAccount([ix]);
} else if (cmd === 'trader-withdraw') {
  console.log('The trading key withdrawing its subaccount\'s collateral (refused here; Phoenix refuses it too)');
  const ix = rise.buildWithdrawFundsIxResolved({ exchange: ex, trader: { authority: tradingKey.toBase58(), traderAccount: child.address.toBase58(), destinationTokenAccount: ata(tradingKey, ex.canonicalMint).address.toBase58() }, amount: 1_000_000n });
  await asTrader([ix]);
} else {
  console.log('usage: see the top of phoenix-perps.mjs');
}
