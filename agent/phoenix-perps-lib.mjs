// Phoenix perps ("Rise", program Etrn…): the exchange's accounts read from
// Solana itself (global configuration, perp asset map, arenas), so nothing
// comes from Phoenix's API. Used by phoenix-perps.mjs.
import * as rise from '@ellipsis-labs/rise';

export const PROGRAM = 'EtrnLzgbS7nMMy5fbD42kXiUzGg8XQzJ972Xtk1cjWih';
export const EMBER = 'EMBERpYNE6ehWmXymZZS2skiFmCa9V5dp14e1iduM5qy';
export const USDC = 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v';

export function client(rpc) {
  const fetchAccount = async (address) => {
    const r = await fetch(rpc, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'getAccountInfo', params: [String(address), { encoding: 'base64' }] }) }).then((r) => r.json());
    const v = r.result?.value;
    if (!v) throw new Error(`no account ${address}`);
    return { address, data: new Uint8Array(Buffer.from(v.data[0], 'base64')), owner: v.owner, lamports: BigInt(v.lamports), exists: true };
  };
  return { fetchAccount, addresses: { phoenixProgramAddress: rise.PHOENIX_PROGRAM_ADDRESS, globalConfigurationAddress: rise.PHOENIX_GLOBAL_CONFIGURATION_ADDRESS, logAuthorityAddress: rise.PHOENIX_LOG_AUTHORITY_ADDRESS } };
}

/** The exchange's accounts, in the shape the SDK's *Resolved builders take. */
export async function exchange(c) {
  if (rise.PHOENIX_PROGRAM_ADDRESS !== PROGRAM || rise.EMBER_PROGRAM_ADDRESS !== EMBER) throw new Error("the SDK names other program addresses");
  const gc = rise.decodeGlobalConfiguration((await c.fetchAccount(rise.PHOENIX_GLOBAL_CONFIGURATION_ADDRESS)).data);
  const req = await rise.fetchRequiredAccounts(c, gc);
  return {
    phoenixProgramAddress: PROGRAM,
    logAuthorityAddress: rise.PHOENIX_LOG_AUTHORITY_ADDRESS,
    globalConfigurationAddress: rise.PHOENIX_GLOBAL_CONFIGURATION_ADDRESS,
    perpAssetMap: gc.perpAssetMapKey,
    globalTraderIndex: req.globalTraderIndexAddresses,
    activeTraderBuffer: req.arenaAddresses,
    canonicalMint: gc.canonicalTokenMintKey,
    usdcMint: USDC,
    globalVault: gc.globalVaultKey,
    withdrawQueue: gc.withdrawQueueKey,
    emberState: rise.EMBER_STATE_ADDRESS,
    emberVault: await rise.getEmberVaultAddress(),
  };
}

/** One market by symbol, from the perp asset map: { symbol, marketAddress, splineCollection, tickSize, baseLotsDecimals }. */
export async function market(c, ex, symbol) {
  const pm = rise.decodePerpAssetMap((await c.fetchAccount(ex.perpAssetMap)).data);
  const e = pm.metadata.entries.find((x) => x.key.toUpperCase() === symbol.toUpperCase());
  if (!e) throw new Error(`no ${symbol} market`);
  const s = e.value.staticMarketParams;
  return { symbol: e.key, marketAddress: s.marketAccount, splineCollection: await rise.getPhoenixSplineCollectionAddress(s.marketAccount), tickSize: s.tickSize, baseLotsDecimals: s.baseLotDecimals, oracleTicks: e.value.oraclePrice.markPrice.price.ticks };
}

export const traderAccount = (authority, sub) => rise.getPhoenixTraderSubaccountAddress({ authority, traderPdaIndex: 0, subaccountIndex: sub });
export const listOf = (b) => b.instructions ?? b.ixs ?? (Array.isArray(b) ? b : [b]);
