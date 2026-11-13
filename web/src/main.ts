import qrcode from 'qrcode-generator';
import { ICONS } from './icons';
import '@fontsource-variable/geist';
import '@fontsource-variable/geist-mono';
import './style.css';
import init, * as core from './core/core.js';
import { createPasskey, onPasskeyDomain, RP_ID, signChallenge, type Passkey } from './passkey';
import { RELAYER, account, formatNear, relayer, setRpc, view, type Config } from './near';
import { hasHistory, history } from './history';

// EVM test chains shown on the dashboard. One address on all of them: the
// MPC signer's derived_public_key for the wallet and path `evm`.
// Testnets of the EVM chains Marvii supports. IDs, RPCs
// and explorers from viem 2.57.8's chain definitions.
const EVM_CHAINS = [
  { id: 84532, name: 'Base', sym: 'ETH', rpc: 'https://sepolia.base.org', explorer: 'https://sepolia.basescan.org/address/' },
  { id: 11155111, name: 'Ethereum', sym: 'ETH', rpc: 'https://11155111.rpc.thirdweb.com', explorer: 'https://sepolia.etherscan.io/address/' },
  { id: 421614, name: 'Arbitrum', sym: 'ETH', rpc: 'https://sepolia-rollup.arbitrum.io/rpc', explorer: 'https://sepolia.arbiscan.io/address/' },
  { id: 11155420, name: 'Optimism', sym: 'ETH', rpc: 'https://sepolia.optimism.io', explorer: 'https://optimism-sepolia.blockscout.com/address/' },
  { id: 97, name: 'BNB Chain', sym: 'BNB', rpc: 'https://data-seed-prebsc-1-s1.bnbchain.org:8545', explorer: 'https://testnet.bscscan.com/address/' },
  { id: 80002, name: 'Polygon', sym: 'POL', rpc: 'https://polygon-amoy.drpc.org', explorer: 'https://amoy.polygonscan.com/address/' },
  { id: 43113, name: 'Avalanche', sym: 'AVAX', rpc: 'https://api.avax-test.network/ext/bc/C/rpc', explorer: 'https://testnet.snowtrace.io/address/' },
  { id: 10143, name: 'Monad', sym: 'MON', rpc: 'https://testnet-rpc.monad.xyz', explorer: 'https://testnet.monadexplorer.com/address/' },
  { id: 998, name: 'HyperEVM', sym: 'HYPE', rpc: 'https://rpc.hyperliquid-testnet.xyz/evm', explorer: '' },
];
// Token balances shown under their chain. Aave V3 Base Sepolia, from
// bgd-labs/aave-address-book (AaveV3BaseSepolia.sol), checked on-chain.
// Other chains, each with its own address from the account's key for that
// chain. Balance sources were checked against live testnet data; chains
// without a working public source show the address only.
type OtherChain = { id: string; name: string; sym: string; decimals: number; explorer?: string; balance?: (addr: string) => Promise<bigint> };
const post = (url: string, body: unknown) =>
  fetch(url, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body), signal: AbortSignal.timeout(8000) }).then((r) => r.json());
const get = (url: string) => fetch(url, { signal: AbortSignal.timeout(8000) }).then((r) => r.json());
/** Bitcoin and Litecoin explorers (mempool.space's API). Calls go through
 *  the relayer first, because some browsers can't reach the explorers
 *  directly (blockers, rate limits); then directly as a fallback. */
const UTXO_DIRECT: Record<string, string> = { bitcoin: 'https://mempool.space/testnet4/api', litecoin: 'https://litecoinspace.org/testnet/api' };
async function utxoGet(chain: string, path: string) {
  try {
    return await relayer<any>(`/utxo/${chain}${path}`);
  } catch {
    return get(UTXO_DIRECT[chain] + (path === '/fees' ? '/v1/fees/recommended' : path));
  }
}
async function utxoBroadcast(chain: string, raw: string): Promise<string> {
  let r: Response;
  try {
    r = await fetch(`${RELAYER}/utxo/${chain}/tx`, { method: 'POST', body: raw, signal: AbortSignal.timeout(20000) });
  } catch {
    // The relayer is unreachable: broadcast directly.
    const d = await fetch(`${UTXO_DIRECT[chain]}/tx`, { method: 'POST', body: raw });
    const t = (await d.text()).trim();
    if (!d.ok) throw new Error(t);
    return t;
  }
  const j = await r.json();
  if (!r.ok) throw new Error(j.error ?? 'broadcast refused');
  return j.txid;
}
const utxoBalance = async (chain: string, a: string) => {
  const j = await utxoGet(chain, `/address/${a}`);
  const c = j.chain_stats, m = j.mempool_stats;
  return BigInt(c.funded_txo_sum - c.spent_txo_sum + m.funded_txo_sum - m.spent_txo_sum);
};
// Stellar's public testnet Horizon (CORS open).
const HORIZON = 'https://horizon-testnet.stellar.org';
// TON Center's public testnet API (CORS open; about one request a second without a key).
const TONCENTER = 'https://testnet.toncenter.com/api/v2';
const relayerBalance = async (chain: string, a: string) => BigInt((await get(`${RELAYER}/chain/${chain}/balance/${encodeURIComponent(a)}`)).balance);
const OTHER_CHAINS: OtherChain[] = [
  { id: 'bitcoin', name: 'Bitcoin', sym: 'BTC', decimals: 8, explorer: 'https://mempool.space/testnet4/address/', balance: (a) => utxoBalance('bitcoin', a) },
  { id: 'solana', name: 'Solana', sym: 'SOL', decimals: 9, explorer: 'https://explorer.solana.com/address/{}?cluster=devnet',
    balance: async (a) => BigInt((await post('https://api.devnet.solana.com', { jsonrpc: '2.0', id: 1, method: 'getBalance', params: [a] })).result.value) },
  { id: 'tron', name: 'TRON', sym: 'TRX', decimals: 6, explorer: 'https://nile.tronscan.org/#/address/',
    balance: async (a) => BigInt((await post('https://nile.trongrid.io/wallet/getaccount', { address: a, visible: true })).balance ?? 0) },
  { id: 'xrp', name: 'XRP', sym: 'XRP', decimals: 6, explorer: 'https://testnet.xrpl.org/accounts/',
    balance: async (a) => {
      const r = (await post(XRPL, { method: 'account_info', params: [{ account: a, ledger_index: 'validated' }] })).result;
      if (r.error === 'actNotFound') return 0n;
      return BigInt(r.account_data.Balance);
    } },
  { id: 'stellar', name: 'Stellar', sym: 'XLM', decimals: 7, explorer: 'https://stellar.expert/explorer/testnet/account/',
    balance: async (a) => {
      const r = await fetch(`${HORIZON}/accounts/${a}`, { signal: AbortSignal.timeout(8000) });
      if (r.status === 404) return 0n; // not created yet: it needs 1 XLM sent to it
      const native = (await r.json()).balances.find((b: any) => b.asset_type === 'native');
      return BigInt(Math.round(Number(native?.balance ?? 0) * 1e7));
    } },
  { id: 'ton', name: 'TON', sym: 'GRAM', decimals: 9, explorer: 'https://testnet.tonviewer.com/',
    balance: async (a) => BigInt((await get(`${TONCENTER}/getAddressBalance?address=${encodeURIComponent(a)}`)).result) },
  { id: 'cardano', name: 'Cardano', sym: 'ADA', decimals: 6, explorer: 'https://preprod.cardanoscan.io/address/',
    // Through the relayer: Koios's POST responses carry no CORS header.
    balance: (a) => relayerBalance('cardano', a) },
  { id: 'litecoin', name: 'Litecoin', sym: 'LTC', decimals: 8, explorer: 'https://litecoinspace.org/testnet/address/', balance: (a) => utxoBalance('litecoin', a) },
  // Through the relayer: Electrum (Bitcoin Cash) and gRPC (Zcash) aren't
  // reachable from a page, and Dogecoin's explorer is relayed the same way.
  { id: 'dogecoin', name: 'Dogecoin', sym: 'DOGE', decimals: 8, explorer: 'https://doge-testnet-explorer.qed.me/address/', balance: (a) => relayerBalance('dogecoin', a) },
  { id: 'bitcoin-cash', name: 'Bitcoin Cash', sym: 'BCH', decimals: 8, explorer: 'https://explorer.selene.cash/address/{}?network=chipnet', balance: (a) => relayerBalance('bitcoin-cash', a) },
  { id: 'zcash', name: 'Zcash', sym: 'ZEC', decimals: 8, explorer: 'https://testnet.zcashexplorer.app/address/', balance: (a) => relayerBalance('zcash', a) },
];

const TOKEN_BALANCES = [
  { chain: 84532, token: '0xba50cd2a20f6da35d788639e581bca8d0b5d4d5f', symbol: 'USDC', decimals: 6 },
  { chain: 84532, token: '0x10f1a9d11cdf50041f3f8cb7191cbe2f31750acc', symbol: 'aUSDC', decimals: 6 },
];


// Tokens the page can name. Anything else is shown by address and raw units.
// Aave's test USDC on Base Sepolia (bgd-labs/aave-address-book, checked on-chain).
const TOKENS: Record<string, { symbol: string; decimals: number }> = {
  '84532:0xba50cd2a20f6da35d788639e581bca8d0b5d4d5f': { symbol: 'USDC', decimals: 6 },
  '84532:0x4200000000000000000000000000000000000006': { symbol: 'WETH', decimals: 18 },
  '560048:0x3508a952176b3c15387c97be809eaffb1982176a': { symbol: 'stETH', decimals: 18 },
  // Polygon's USDC.e (Polymarket's collateral) and Base's USDC (top-ups).
  '137:0x2791bca1f2de4661ed88a30c99a7a9449aa84174': { symbol: 'USDC.e', decimals: 6 },
  '8453:0x833589fcd6edb6e08f4c7c32d4f71b54bda02913': { symbol: 'USDC', decimals: 6 },
};
/** Hyperliquid testnet perps by asset index (`meta` universe, checked
 *  9 November 2026). */
const HL_MARKETS: Record<number, string> = { 0: 'SOL', 3: 'BTC', 4: 'ETH', 54: 'NEAR' };
const hlName = (a: number) => (HL_MARKETS[a] ? `${HL_MARKETS[a]} perps` : `market #${a}`);
/** Cap amounts in plain words where the page knows the asset. */
/** The payment-only chains agents may pay on (Dogecoin's signatures don't
 *  commit to amounts, so its fees can't be capped: not offered). */
const PAY_CHAINS: Record<string, string> = { bitcoin: 'BTC', litecoin: 'LTC', 'bitcoin-cash': 'BCH', zcash: 'ZEC', dogecoin: 'DOGE', cardano: 'ADA' };
const PAY_NAMES: Record<string, string> = { bitcoin: 'Bitcoin', litecoin: 'Litecoin', 'bitcoin-cash': 'Bitcoin Cash', zcash: 'Zcash', dogecoin: 'Dogecoin', cardano: 'Cardano' };
/** Decimals of each payment chain's coin (ADA has 6, the rest 8). */
const payDecimals = (chain: string) => (chain === 'cardano' ? 6 : 8);
function capWords(asset: string, daily: string): string {
  if (asset === 'hyperliquid:usd') return `$${esc(fmtUnits(BigInt(daily), 6, 2))} of Hyperliquid orders`;
  if (asset.startsWith('utxo:') && PAY_CHAINS[asset.slice(5)]) return `${esc(fmtUnits(BigInt(daily), payDecimals(asset.slice(5)), 8))} ${PAY_CHAINS[asset.slice(5)]} (fees included)`;
  const coins: Record<string, [number, string]> = { near: [24, 'NEAR'], sol: [9, 'SOL'], ton: [9, 'GRAM'], xlm: [7, 'XLM'], xrp: [6, 'XRP'], trx: [6, 'TRX'], 'nep141:wrap.testnet': [24, 'wNEAR'], 'nep141:wrap.near': [24, 'wNEAR'] };
  if (coins[asset]) return `${esc(fmtUnits(BigInt(daily), coins[asset][0]))} ${coins[asset][1]}`;
  const evmCoin = asset.match(/^evm:(\d+)$/);
  if (evmCoin) return `${esc(fmtUnits(BigInt(daily), 18))} ${esc(EVM_CHAINS.find((c) => c.id === Number(evmCoin[1]))?.sym ?? 'ETH')} on ${esc(chainName(Number(evmCoin[1])))}`;
  if (asset.startsWith('phoenix:')) return `${perpSize(asset.slice(8), daily)} traded on Phoenix perps`;
  if (asset === 'polymarket:sell') return `sell up to ${esc(fmtUnits(BigInt(daily), 6, 2))} Polymarket shares, at any price`;
  if (asset === 'nft:ton') return `${esc(daily)} NFT transfer${daily === '1' ? '' : 's'} on TON`;
  if (asset.startsWith('nft:')) return `${esc(daily)} NFT listing${daily === '1' ? '' : 's'} on ${esc(asset.slice(4).replace(/^evm:/, 'chain ').replace('solana', 'Solana'))}`;
  const be = asset.match(/^borrow:evm:(\d+):(0x[0-9a-fA-F]{40})$/);
  if (be) return `borrowing ${amount(Number(be[1]), be[2].toLowerCase(), daily).replace(/<\/?b>/g, '')} on ${esc(chainName(Number(be[1])))}`;
  const bk = KNOWN_TOKENS[asset.replace(/^borrow:solana:/, 'spl:')];
  if (asset.startsWith('borrow:solana:') && bk) return `borrowing ${esc(fmtUnits(BigInt(daily), bk[0]))} ${bk[1]}`;
  if (asset.startsWith('borrow:')) return `borrowing ${esc(daily)} units of ${esc(asset.split(':').slice(2).join(':'))} (${esc(asset.split(':')[1])})`;
  const known = KNOWN_TOKENS[asset];
  if (known) return `${esc(fmtUnits(BigInt(daily), known[0]))} ${known[1]}`;
  return `${esc(daily)} of ${esc(asset)}`;
}
/** Tokens on non-EVM chains with a known name and decimals (each checked at its mint or master). */
const KNOWN_TOKENS: Record<string, [number, string]> = {
  'spl:EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v': [6, 'USDC on Solana'],
  'jetton:kQAiQ2XK7BXePLwemeo-u4wNyjg-wxGeySmaFGEP7R2Mhf4m': [9, 'tsTON'],
};
const POOLS: Record<string, string> = {
  '84532:0x8bab6d1b75f19e9ed9fce8b9bd338844ff79ae27': 'Aave V3 pool',
  '84532:0x94cc0aac535ccdb3c01d6787d6413c739ae12bc4': 'Uniswap V3 router',
  // Polymarket's exchanges on Polygon (getCollateral() is USDC.e): the agent may sign orders there.
  '137:0x4bfb41d5b3570defd03c39a9a4d8de6bd8b8982e': 'Polymarket (orders)',
  '137:0xc5d563a36ae78145c45a50134d48a1215220f80a': 'Polymarket neg-risk (orders)',
  // Lido on Hoodi (docs.lido.fi/deployed-contracts/hoodi, checked on-chain).
  '560048:0x3508a952176b3c15387c97be809eaffb1982176a': 'Lido stETH',
  '560048:0x7e99ee3c66636de415d2d7c880938f2f40f94de4': 'Lido wstETH',
  '560048:0xfe56573178f1bcdf53f01a6e9977670dcbbd9186': 'Lido withdrawal queue',
  // Seaport 1.6 (information() reports "1.6" on Base Sepolia).
  '84532:0x0000000000000068f116a894984e2db1123eb395': 'Seaport (OpenSea)',
};
/** Venue functions by selector (keccak256 of the signature). The extension
 *  checks where each one's result goes (v5). */
const FN_NAMES: Record<string, string> = {
  '0x617ba037': 'supply', '0x69328dec': 'withdraw', '0x573ade81': 'repay', '0xa415bcad': 'borrow', '0x04e45aaf': 'swap',
  '0xa1903eab': 'stake', '0xea598cb0': 'wrap', '0xde0e9a3e': 'unwrap', '0xd6681042': 'request withdrawal', '0x19aa6257': 'request withdrawal (wstETH)', '0xf8444436': 'claim withdrawal',
  '0xfb0f3ee1': 'buy', '0x00000000': 'buy', '0xfd9f1e10': 'cancel listings', '0xa22cb465': 'approve NFTs for',
  '0xa6dfcf86': 'cancel orders, and sign orders', '0xd0e30db0': 'deposit', '0x2e1a7d4d': 'withdraw', '0x0ba7d4a7': 'withdraw exact', '0x4e71d92d': 'claim', '0xd1058e59': 'claim all',
  '0x1249c58b': 'supply', '0xdb006a75': 'redeem', '0x852a12e3': 'redeem', '0xc5ebeaec': 'borrow', '0x0e752702': 'repay', '0xc2998238': 'use as collateral', '0x7ff36ab5': 'swap',
};
/** TRON contracts by name (Nile): JustLend's from its contracts.json, SunSwap's from docs.sun.io. */
const TRON_NAMES: Record<string, string> = {
  TZ8du1HkatTWDbS6FLZei4dQfjfpSm9mxp: 'sTRX (JustLend staked TRX)', TKM7w4qFmkXQLEF2MgrQroBYpd5TY7i1pq: 'JustLend jTRX', TJUCStq3WqfKqZLuZje5v7z6Ua6iBry1P6: 'JustLend', TYMjxCXfqLpMWW1QToP6hbcjpion7EE25p: 'SunSwap V2',
};
/** Venues the grant form offers by name. Aave V3 from bgd-labs/aave-address-book
 *  (checked on-chain); Uniswap V3 SwapRouter02 on Base Sepolia, whose
 *  factory() and WETH9() were checked on-chain. */
const VENUES = [
  { id: 'aave', name: 'Aave V3', what: 'supply, withdraw, repay', chain: 84532, to: '0x8bab6d1b75f19e9ed9fce8b9bd338844ff79ae27', selectors: ['0x617ba037', '0x69328dec', '0x573ade81'] },
  { id: 'uniswap', name: 'Uniswap V3', what: 'swap', chain: 84532, to: '0x94cc0aac535ccdb3c01d6787d6413c739ae12bc4', selectors: ['0x04e45aaf'] },
];
const fnName = (sel: string) => FN_NAMES[sel.toLowerCase()] ?? `function ${sel}`;
const chainName = (id: number) => EVM_CHAINS.find((c) => c.id === id)?.name ?? ({ 560048: 'Hoodi' } as Record<number, string>)[id] ?? `chain ${id}`;
/** A long list folded after the first few. */
function fold(items: string[], first = 6): string {
  if (items.length <= first) return items.join('; ');
  return `${items.slice(0, first).join('; ')} <details class="inline"><summary>and ${items.length - first} more</summary>${items.slice(first).join('; ')}</details>`;
}
function amount(chain: number, token: string, raw: string) {
  const t = TOKENS[`${chain}:${token}`];
  if (!t) return `${esc(raw)} units of token ${esc(token)}`;
  const n = BigInt(raw);
  const d = 10n ** BigInt(t.decimals);
  const frac = (n % d).toString().padStart(t.decimals, '0').replace(/0+$/, '');
  return `<b>${n / d}${frac ? '.' + frac : ''} ${esc(t.symbol)}</b>`;
}
/** Addresses as 0x1234…abcd (brand guide); the full one is copied on tap. */
const short = (a: string) => (a.length > 14 ? `${a.slice(0, 6)}…${a.slice(-4)}` : a);
const copyable = (a: string) => `<button class="addr mono" data-copy="${esc(a)}" title="Copy">${esc(short(a))}</button>`;
/** A name in full (not shortened), copyable. */
const copyableName = (a: string) => `<button class="addr mono" data-copy="${esc(a)}" title="Copy">${esc(a)}</button>`;

// ---- Names: alice.marvii.testnet, a deposit address that forwards here ----

const RESERVED_NAMES = ['marvii', 'admin', 'support', 'help', 'official', 'team', 'wallet', 'near', 'security', 'relayer'];
/** The registry's rules, checked here first so a bad name never costs a tap. */
function nameProblem(n: string): string | null {
  if (!/^[a-z0-9]+([-_][a-z0-9]+)*$/.test(n) || n.length < 2 || n.length > 32) return 'Use 2–32 lowercase letters and digits, with single - or _ between them.';
  if (RESERVED_NAMES.includes(n) || n.startsWith('0x') || n.startsWith('0s')) return 'That name is reserved.';
  return null;
}
async function nameAvailable(n: string): Promise<boolean> {
  return (await view<string | null>(cfg.names!, 'wallet_of', { name: n })) === null;
}
const registerMessage = (n: string) => core.call_message(cfg.chain_id, wallet, nonce(), createdAt(), cfg.names!, 'register', JSON.stringify({ name: n }), '0', BigInt(60));

/** Live feedback on a name field: format, then availability. */
function watchName(input: HTMLInputElement, out: HTMLElement) {
  let t: ReturnType<typeof setTimeout> | undefined;
  input.oninput = () => {
    const n = input.value.trim();
    clearTimeout(t);
    if (!n) return void (out.textContent = '');
    const p = nameProblem(n);
    if (p) return void (out.textContent = p);
    out.textContent = 'Checking…';
    t = setTimeout(() => nameAvailable(n).then((ok) => (out.textContent = ok ? `${n}.${cfg.names} is available` : 'That name is taken')).catch(() => (out.textContent = '')), 300);
  };
}

/** For an account without a name: choose one, approved with the passkey. */
function nameForm() {
  const m = document.getElementById('modal')!;
  m.innerHTML = `
    <div class="backdrop"><form class="card sheet" id="send">
      <h2>Choose a name</h2>
      <p class="small muted">Anyone can send NEAR and tokens to your name, from any wallet or exchange. It forwards everything to this account. A name can't be changed later.</p>
      <label>Name <span class="row"><input name="name" required class="mono" autocomplete="off" autocapitalize="none" spellcheck="false" placeholder="alice"><span class="muted mono">.${esc(cfg.names)}</span></span></label>
      <p class="small muted" id="name-note"></p>
      <div class="row end"><button type="button" class="secondary" id="cancel">Cancel</button><button class="primary">Review</button></div>
      <div id="status" class="status"></div>
    </form></div>`;
  const input = m.querySelector<HTMLInputElement>('input[name=name]')!;
  watchName(input, document.getElementById('name-note')!);
  document.getElementById('cancel')!.onclick = () => (m.innerHTML = '');
  (document.getElementById('send') as HTMLFormElement).onsubmit = async (ev) => {
    ev.preventDefault();
    try {
      const n = input.value.trim();
      const p = nameProblem(n);
      if (p) throw new Error(p);
      if (!(await nameAvailable(n))) throw new Error('That name is taken');
      confirm(registerMessage(n), { after: async () => `Your name is ${copyableName(`${n}.${cfg.names}`)}` });
    } catch (e) {
      status(String((e as Error).message ?? e), 'error');
    }
  };
}

let nearBalance = '0';
/** Sends NEAR to any account or name; the passkey approves the exact transfer. */
function nearSendForm() {
  const m = document.getElementById('modal')!;
  m.innerHTML = `
    <div class="backdrop"><form class="card sheet" id="send">
      <h2>Send on NEAR</h2>
      <p class="small muted">Balance ${esc(formatNear(nearBalance))} NEAR. Keep a little for your account's storage.</p>
      <label>To <input name="to" required class="mono" autocomplete="off" autocapitalize="none" spellcheck="false" placeholder="alice.testnet"></label>
      <label>Amount (NEAR) <input name="amount" required inputmode="decimal" placeholder="0.0"></label>
      <div class="row end"><button type="button" class="secondary" id="cancel">Cancel</button><button class="primary">Review</button></div>
      <div id="status" class="status"></div>
    </form></div>`;
  document.getElementById('cancel')!.onclick = () => (m.innerHTML = '');
  (document.getElementById('send') as HTMLFormElement).onsubmit = async (ev) => {
    ev.preventDefault();
    try {
      const f = new FormData(ev.target as HTMLFormElement);
      const to = String(f.get('to')).trim().toLowerCase();
      const yocto = toUnits(String(f.get('amount')), 24);
      if (BigInt(yocto) > BigInt(nearBalance)) throw new Error('More than your balance');
      if (!(await account(to))) throw new Error('No NEAR account by that name');
      const msg = core.transfer_message(cfg.chain_id, wallet, nonce(), createdAt(), to, yocto);
      confirm(msg, { after: async (out) => `Sent. <a target="_blank" rel="noopener" href="https://testnet.nearblocks.io/txns/${esc(out.tx_hash)}">View transaction</a>` });
    } catch (e) {
      status(String((e as Error).message ?? e), 'error');
    }
  };
}

/** The NEAR row: the name as the address people send to, and the account. */
async function fillNear() {
  const [a, name] = await Promise.all([
    account(wallet),
    cfg.names ? view<string | null>(cfg.names, 'name_of', { wallet }).catch(() => null) : Promise.resolve(null),
  ]);
  const bal = `${a ? formatNear(a.amount) : '0.0000'} NEAR`;
  const d = document.getElementById('chain-near');
  if (!d) return;
  d.querySelector('.bal')!.textContent = bal;
  const rows = name
    ? `<div class="row between small"><span class="muted">Your NEAR address</span><span>${copyableName(name)}</span></div>
       <div class="row between small"><span class="muted">Account</span><span>${copyable(wallet)}</span></div>`
    : `<div class="row between small"><span class="muted">Address</span><span>${copyable(wallet)}</span></div>`;
  const choose = !name && cfg.names ? `<div class="row end"><button class="secondary small-btn" data-name="1">Choose a name</button></div>` : '';
  deposits['chain-near'] = { address: wallet, name: name ?? undefined, explorer: `https://testnet.nearblocks.io/address/${wallet}` };
  const an = document.getElementById('acct-name');
  if (an) an.innerHTML = name ? copyableName(name) : copyable(wallet);
  const tb = document.querySelector('#tile-chain-near .bal');
  if (tb) tb.textContent = bal;
  nearBalance = a ? a.amount : '0';
  setActions('chain-near', 'near');
  const icon = `<span class="coin small" aria-hidden="true">${ICONS['chain-near'] ?? ''}</span>`;
  d.querySelector('.assets')!.innerHTML = `<div class="row between asset"><span class="chain-name">${icon}NEAR</span><span>${esc(bal.split(' ')[0])}</span></div>` + rows + choose;
  if (name) {
    // Deposits sent to the name wait there until forwarded; ask now rather
    // than wait for the relayer's next round.
    relayer<{ forwarded: boolean }>('/names/forward', { name })
      .then((r) => (r.forwarded ? fillNear() : undefined))
      .catch(() => {});
  }
}

const named = (chain: number, addr: string) => (POOLS[`${chain}:${addr}`] ? `${esc(POOLS[`${chain}:${addr}`])} ` : '') + copyable(addr);

const app = document.getElementById('app')!;
let cfg: Config;
let pk: Passkey | null = null;
let wallet = '';

const esc = (s: unknown) =>
  String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);

const store = {
  get passkey(): Passkey | null {
    try {
      return JSON.parse(localStorage.getItem('marvii-near:passkey') ?? 'null');
    } catch {
      return null;
    }
  },
  set passkey(v: Passkey | null) {
    try {
      localStorage.setItem('marvii-near:passkey', JSON.stringify(v));
    } catch {}
  },
};

function render(html: string) {
  app.innerHTML = html;
}

function status(msg: string, kind: 'info' | 'error' | 'ok' = 'info') {
  // An open sheet's own status line first, so messages show where you are.
  const el = document.querySelector<HTMLElement>('.sheet .status') ?? document.getElementById('status');
  if (el) {
    el.className = `status ${kind}`;
    el.textContent = msg;
  }
}

// ---- The loader: an animated Marvii mark over the page while we wait ----

const MARK = `<svg class="mark" viewBox="0 0 64 64" aria-hidden="true"><path d="M11 52V30a10.5 10.5 0 0 1 21 0v22M32 52V30a10.5 10.5 0 0 1 21 0v22" pathLength="100" fill="none" stroke="currentColor" stroke-width="8" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
/** Shows the loader with `text`, or hides it with null. */
function busy(text: string | null) {
  let el = document.getElementById('busy');
  if (text === null) return void el?.remove();
  if (!el) {
    el = document.createElement('div');
    el.id = 'busy';
    el.className = 'busy';
    el.setAttribute('role', 'status');
    el.setAttribute('aria-live', 'polite');
    el.innerHTML = `<div class="busy-card">${MARK}<p></p></div>`;
    document.body.appendChild(el);
  }
  el.querySelector('p')!.textContent = text;
}
/** Rejects after `ms`, so nothing waits forever. */
function within<T>(p: Promise<T>, ms: number): Promise<T> {
  return Promise.race([p, new Promise<T>((_, no) => setTimeout(() => no(new Error('timeout')), ms))]);
}
/** Calls `check` every 2 s until it returns something, for up to `ms`. */
async function until<T>(check: () => Promise<T | null | undefined>, ms: number): Promise<T | null> {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const v = await check().catch(() => null);
    if (v) return v;
    await new Promise((r) => setTimeout(r, 2000));
  }
  return null;
}
/** Back to the wallet after an action; if that fails, reload the page. */
/** Shows the wallet. dashboard() draws the page at once, then fills in
 *  balances and the agent lists as they arrive: if those are slow (a rate-
 *  limited RPC), stop waiting and let them land. Reloading here would start
 *  over and could loop for as long as the RPC stays slow. */
async function backToWallet() {
  await within(dashboard(), 30000).catch(() => {});
}

async function main() {
  await init();
  cfg = await relayer<Config>('/config');
  setRpc(cfg.rpc);
  // Read-only view of any wallet: ?wallet=0s… (no passkey, no buttons).
  let viewOnly = new URLSearchParams(location.search).get('wallet');
  // A name works too: ?wallet=alice.marvii.testnet
  if (viewOnly && cfg.names && viewOnly.endsWith(`.${cfg.names}`)) {
    viewOnly = await view<string | null>(cfg.names, 'wallet_of', { name: viewOnly }).catch(() => null);
  }
  if (viewOnly && /^0s[0-9a-f]{40}$/.test(viewOnly)) {
    wallet = viewOnly;
    document.body.classList.add('readonly');
    await dashboard();
    return;
  }
  pk = store.passkey;
  if (pk) {
    // The address comes from the extension the wallet was created with; it
    // keeps that address when the passkey moves it to a newer extension.
    wallet = (pk as Passkey & { wallet?: string }).wallet ?? core.wallet_account_id(cfg.wallet_code_hash, pk.publicKey, cfg.extension);
    busy('Opening your wallet…');
    await backToWallet();
    busy(null);
  } else {
    start();
  }
}

function start() {
  render(`
    <section class="card hero">
      <h1>One passkey, every chain</h1>
      <p>No seed phrase, no password. Give agents and apps limited, revocable permission to act for you, and approve anything outside it with one tap.</p>
      ${cfg.names ? `<label class="small">Choose your name <span class="row"><input id="name" class="mono" autocomplete="off" autocapitalize="none" spellcheck="false" placeholder="alice"><span class="muted mono">.${esc(cfg.names)}</span></span></label><p class="small muted" id="name-note">Anyone can send to it from any wallet or exchange. Optional; you can choose one later.</p>` : ''}
      <button id="create" class="primary"${onPasskeyDomain() ? '' : ' disabled'}>Create with a passkey</button>
      ${onPasskeyDomain() ? '' : `<p class="notice">Marvii passkeys belong to <a href="https://${esc(RP_ID)}">${esc(RP_ID)}</a>. Create and use your wallet there.</p>`}
      <p class="muted small">Testnet only. Unaudited.</p>
      <div id="status" class="status"></div>
    </section>`);
  const nameInput = document.getElementById('name') as HTMLInputElement | null;
  if (nameInput) watchName(nameInput, document.getElementById('name-note')!);
  document.getElementById('create')!.onclick = async () => {
    try {
      const chosen = nameInput?.value.trim() ?? '';
      if (chosen) {
        const p = nameProblem(chosen);
        if (p) throw new Error(p);
        if (!(await nameAvailable(chosen))) throw new Error('That name is taken. Try another.');
      }
      busy('Creating your passkey…');
      const p = await createPasskey(`Marvii ${new Date().toISOString().slice(0, 10)}`);
      pk = p;
      wallet = core.wallet_account_id(cfg.wallet_code_hash, p.publicKey, cfg.extension);
      store.passkey = { ...p, wallet } as Passkey;
      busy('Activating your wallet: approve with your passkey once more…');
      // With a name, the same tap that activates the wallet registers it.
      const msg = chosen ? registerMessage(chosen) : core.empty_message(cfg.chain_id, wallet, nonce(), createdAt());
      // Phones can drop the connection while the passkey sheet is open, so
      // whatever the relay call says, wait for the account itself.
      const out = await within(signAndRelay(msg), 90000).catch(() => null);
      busy('Setting up your wallet…');
      const created = await until(() => account(wallet), 60000);
      if (!created) throw new Error('Activation did not go through. Please try again.');
      // The relayer's gift towards grant deposits; it can say no (daily
      // limits). The wallet works without it, but granting needs some NEAR.
      const fundError = await within(relayer('/wallet/fund', { public_key: p.publicKey, wallet }), 30000).then(() => null, (e) => String((e as Error).message ?? e));
      busy('Opening your wallet…');
      await backToWallet();
      busy(null);
      if (fundError && fundError !== 'already funded') status(`Your account is ready. The free testnet NEAR wasn't sent (${fundError}). To give apps and agents permission, deposit about 0.1 NEAR from the NEAR row first.`, 'error');
      const named = chosen && cfg.names ? await view<string | null>(cfg.names, 'name_of', { wallet }).catch(() => null) : null;
      if (chosen && !named) status(`Your account is ready, but the name ${chosen} couldn't be registered. Choose a name from the NEAR row.`, 'error');
    } catch (e) {
      busy(null);
      status(String((e as Error).message ?? e), 'error');
    }
  };
}

const nonce32 = () => crypto.getRandomValues(new Uint32Array(1))[0];
const nonce = () => crypto.getRandomValues(new Uint32Array(1))[0];
// A minute early: the wallet refuses messages dated after the block time.
const createdAt = () => BigInt(Math.floor(Date.now() / 1000) - 60);

type Outcome = { tx_hash: string; ok: boolean; failures: unknown[]; logs: string[] };

/** Has the passkey sign this exact message and sends it through the relayer. */
async function signAndRelay(msgJson: string, pendingId?: number): Promise<Outcome> {
  const msg = core.normalize_request(msgJson);
  const challenge = core.request_hash(msg);
  const a = await signChallenge(pk!, challenge);
  const proof = core.make_proof(a.authenticatorData, a.clientDataJSON, a.signature);
  if (!core.verify_proof(pk!.publicKey, msg, proof)) throw new Error('The passkey signature does not verify');
  // The same signed message can be sent again without another tap: if an
  // earlier attempt did land, the wallet refuses the repeat (same nonce).
  // Errors that can never succeed aren't retried.
  const permanent = /does not verify|not a Marvii wallet|wrong chain_id|nonce already used|expired/i;
  for (let attempt = 1; ; attempt++) {
    try {
      return await relayer<Outcome>('/wallet/relay', { public_key: pk!.publicKey, msg: JSON.parse(msg), proof, pending_id: pendingId });
    } catch (e) {
      if (attempt >= 3 || permanent.test(String((e as Error).message))) throw e;
      await new Promise((r) => setTimeout(r, 3000 * attempt));
    }
  }
}

async function evmAddress(): Promise<string> {
  const k = await view<string>(cfg.signer, 'derived_public_key', { path: 'evm', predecessor: wallet, domain_id: 0 });
  return core.evm_address(k);
}

async function evmRpc(rpcUrl: string, method: string, params: unknown[]) {
  const r = await fetch(rpcUrl, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
    // A chain whose RPC hangs shows "–" rather than loading forever.
    signal: AbortSignal.timeout(8000),
  }).then((r) => r.json());
  if (r.error || typeof r.result !== 'string') throw new Error('rpc');
  return BigInt(r.result);
}

const fmtUnits = (n: bigint, decimals: number, shown = 4): string => {
  const d = 10n ** BigInt(decimals);
  return `${n / d}.${((n % d) * 10n ** BigInt(shown) / d).toString().padStart(shown, '0')}`;
};

async function evmBalance(rpcUrl: string, addr: string, sym: string): Promise<string> {
  return `${fmtUnits(await evmRpc(rpcUrl, 'eth_getBalance', [addr, 'latest']), 18)} ${sym}`;
}

async function tokenBalance(rpcUrl: string, token: string, addr: string, decimals: number): Promise<string> {
  // balanceOf(address) = 0x70a08231 (cast sig)
  const data = '0x70a08231' + addr.slice(2).toLowerCase().padStart(64, '0');
  return fmtUnits(await evmRpc(rpcUrl, 'eth_call', [{ to: token, data }, 'latest']), decimals, 2);
}

type Session = {
  grant: { label: string; public_key: string; expires_at_ms: number; inactivity_ms: number; caps: { asset: string; daily: string }[]; evm_venues: { chain_id: number; to: string; selector: string }[]; near_venues: { receiver_id: string; method: string }[]; hl_markets?: { asset: number; max_leverage: number }[] };
  nonce: number;
  last_used_ms: number;
  revoked: boolean;
};

/** What the Deposit sheet shows for each row. */
const deposits: Record<string, { address: string; name?: string; explorer?: string }> = {};
const ARROW_UP = `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 19V5M5 12l7-7 7 7" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
const ARROW_DOWN = `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 5v14M5 12l7 7 7-7" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
/** Send and Receive, at the top of a chain's page. */
function setActions(id: string, sendChain?: number | string) {
  const el = document.querySelector(`#${id} .acts`);
  if (!el) return;
  const send = sendChain !== undefined
    ? `<button class="act" data-send="${sendChain}"><span class="act-ic">${ARROW_UP}</span>Send</button>`
    : `<button class="act" disabled title="Not yet on this chain"><span class="act-ic">${ARROW_UP}</span>Send</button>`;
  el.innerHTML = `${send}<button class="act" data-deposit="${id}"><span class="act-ic">${ARROW_DOWN}</span>Receive</button>`;
}

/** Deposit: the address to send to, as text and a QR code. */
function depositSheet(id: string) {
  const d = deposits[id];
  if (!d?.address) return;
  const chain = document.querySelector(`#${id} h1`)?.textContent ?? '';
  const shown = d.name ?? d.address;
  const qr = qrcode(0, 'M');
  qr.addData(shown);
  qr.make();
  const evm = EVM_CHAINS.some((c) => `chain-${c.id}` === id);
  const note = id === 'chain-near'
    ? d.name ? 'Send NEAR or tokens to this name from any wallet or exchange. It forwards everything to your account.' : 'Some NEAR wallets don\'t accept this address yet. Choose a name in the NEAR row for one that works everywhere.'
    : evm ? `The same address works on every EVM chain here. Send only on ${esc(chain)} test network.` : `Send only ${esc(chain)} test network coins to this address.`;
  document.getElementById('modal')!.innerHTML = `
    <div class="backdrop"><div class="card sheet deposit">
      <h2>Deposit on ${esc(chain)}</h2>
      <div class="qr">${qr.createSvgTag({ cellSize: 4, margin: 2, scalable: true })}</div>
      <p class="mono addr-full">${esc(shown)}</p>
      ${d.name ? `<p class="small muted">Account ${esc(d.address)}</p>` : ''}
      <p class="small muted">${note}</p>
      <div class="row end"><button class="secondary" id="cancel">Close</button><button class="primary" data-copy="${esc(shown)}">Copy address</button></div>
    </div></div>`;
  document.getElementById('cancel')!.onclick = () => (document.getElementById('modal')!.innerHTML = '');
}

/** A chain's tile on the Assets page. */
function chainTile(id: string, name: string) {
  const icon = ICONS[id] ? `<span class="coin" aria-hidden="true">${ICONS[id]}</span>` : '';
  return `<a class="chain-tile" id="tile-${id}" href="#/${id}">${icon}<span class="tile-name">${esc(name)}</span><span class="bal muted">…</span></a>`;
}

/** A chain's own page: balance, Send and Receive, what it holds, history. */
function chainView(id: string, name: string) {
  const icon = ICONS[id] ? `<span class="coin big-coin" aria-hidden="true">${ICONS[id]}</span>` : '';
  return `<section class="view" data-view="${id}" id="${id}">
    <a class="back" href="#/assets">‹ Assets</a>
    <div class="card chain-head">
      ${icon}<h1>${esc(name)}</h1>
      <div class="big bal">…</div>
      <div class="acts"></div>
    </div>
    <div class="card"><h2>Assets</h2><div class="assets"><p class="muted small">Loading…</p></div></div>
    ${id === 'chain-998' ? HL_CARD : ''}
    <div class="card"><h2>History</h2><ul class="list history" data-history="${id}"><li class="muted small">Open to load.</li></ul></div>
  </section>`;
}

/** Fills a chain row: the native balance on the row, and in its dropdown the
 *  address and every asset held there. */
function fillChain(id: string, address: string, native: string, assets: string[], explorer?: string, sendChain?: number | string) {
  const d = document.getElementById(id);
  if (!d) return;
  d.querySelector('.bal')!.textContent = native;
  const tile = document.querySelector(`#tile-${id} .bal`);
  if (tile) tile.textContent = native;
  const link = explorer ? ` <a class="small" target="_blank" rel="noopener" href="${esc(explorer)}">Explorer</a>` : '';
  deposits[id] = { address, explorer };
  setActions(id, sendChain);
  const sym = native.split(' ').pop()!;
  const icon = ICONS[id] ? `<span class="coin small" aria-hidden="true">${ICONS[id]}</span>` : '';
  d.querySelector('.assets')!.innerHTML =
    [`<div class="row between asset"><span class="chain-name">${icon}${esc(sym)}</span><span>${esc(native.split(' ')[0])}</span></div>`, ...assets].join('') +
    `<div class="row between small addr-row"><span class="muted">Address</span><span>${copyable(address)}${link}</span></div>`;
}

// ---- Views: Home, Assets (a page per chain), Swap, Apps & agents ----

const VIEW_TITLES: Record<string, string> = { home: 'Marvii', assets: 'Assets', swap: 'Swap', topup: 'Top up', agents: 'Apps & agents' };
/** A button that opens a menu of links (the Swap tile, the protocol picker). */
function menuButton(btnId: string, menuId: string) {
  const btn = document.getElementById(btnId) as HTMLButtonElement;
  const menu = document.getElementById(menuId)!;
  const set = (open: boolean) => {
    menu.hidden = !open;
    btn.setAttribute('aria-expanded', String(open));
  };
  btn.onclick = (ev) => {
    ev.stopPropagation();
    set(menu.hidden);
    if (!menu.hidden) menu.querySelector<HTMLElement>('a')?.focus();
  };
  document.addEventListener('click', (ev) => {
    if (!menu.hidden && !menu.contains(ev.target as Node)) set(false);
  });
  document.addEventListener('keydown', (ev) => {
    if (menu.hidden) return;
    if (ev.key === 'Escape') {
      set(false);
      btn.focus();
    } else if (ev.key === 'ArrowDown' || ev.key === 'ArrowUp') {
      ev.preventDefault();
      const items = Array.from(menu.querySelectorAll<HTMLElement>('a'));
      const i = items.indexOf(document.activeElement as HTMLElement);
      items[(i + (ev.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length]?.focus();
    }
  });
  window.addEventListener('hashchange', () => set(false));
}

/** Shows the view the URL's hash names (#/assets, #/chain-84532, …). */
function route() {
  const want = location.hash.replace(/^#\/?/, '') || 'home';
  const views = Array.from(document.querySelectorAll<HTMLElement>('.view'));
  const v = views.find((x) => x.dataset.view === want) ?? views.find((x) => x.dataset.view === 'home');
  views.forEach((x) => x.classList.toggle('on', x === v));
  const proto = PROTOCOLS.find((p) => `p-${p.id}` === v?.dataset.view);
  document.title = proto ? `${proto.name} · Marvii` : VIEW_TITLES[v?.dataset.view ?? ''] ? `${VIEW_TITLES[v!.dataset.view!]} · Marvii` : 'Marvii';
  if (v?.dataset.view?.startsWith('chain-')) loadHistory(v.dataset.view).catch(() => {});
  window.scrollTo(0, 0);
}
window.addEventListener('hashchange', route);

/** A chain's recent transactions, loaded when its page opens. */
async function loadHistory(id: string) {
  const ul = document.querySelector<HTMLElement>(`[data-history="${id}"]`);
  if (!ul || ul.dataset.loaded) return;
  const chain = id.replace(/^chain-/, '');
  const d = deposits[id];
  if (!d?.address && chain !== 'near') {
    ul.innerHTML = '<li class="muted small">Loading…</li>';
    await until(async () => deposits[id]?.address, 20000);
  }
  const addr = deposits[id]?.address ?? '';
  const explorer = deposits[id]?.explorer;
  const more = explorer ? `<li class="small"><a target="_blank" rel="noopener" href="${esc(explorer)}">Everything on the explorer ›</a></li>` : '';
  if (!hasHistory(chain)) {
    ul.innerHTML = explorer ? `<li class="muted small">This chain's history is on its explorer.</li>${more}` : '<li class="muted small">No history source for this chain yet.</li>';
    ul.dataset.loaded = '1';
    return;
  }
  ul.innerHTML = '<li class="muted small">Loading…</li>';
  try {
    const items = await history(chain, addr, wallet);
    ul.dataset.loaded = '1';
    ul.innerHTML = (items.length
      ? items.map((t) => `<li class="hist"><div class="row between"><span>${esc(t.label)}${t.ok === false ? ' <span class="pill bad">failed</span>' : ''}</span><a class="small mono" target="_blank" rel="noopener" href="${esc(t.link)}">${esc(t.hash.slice(0, 8))}…</a></div>${t.time ? `<div class="small muted">${esc(new Date(t.time).toLocaleString())}</div>` : ''}</li>`).join('')
      : '<li class="muted small">Nothing yet.</li>') + more;
  } catch {
    ul.innerHTML = `<li class="muted small">History is unavailable right now.</li>${more}`;
  }
}

const ICON_ASSETS = `<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="6" width="18" height="13" rx="3" fill="none" stroke="currentColor" stroke-width="1.8"/><path d="M3 10h18M16 14.5h2" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg>`;
const ICON_SWAP = `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 4v13M7 17l-3-3M7 17l3-3M17 20V7M17 7l-3 3M17 7l3 3" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
const ICON_AGENTS = `<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="5" y="8" width="14" height="11" rx="3" fill="none" stroke="currentColor" stroke-width="1.8"/><path d="M12 8V5M9.5 13h.01M14.5 13h.01M9 16.5h6" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/><circle cx="12" cy="4" r="1.2" fill="currentColor"/></svg>`;

const HL_CARD = `<section class="card" id="hl-card" hidden>
      <div class="row between"><h2>Hyperliquid</h2><span class="pill">testnet</span></div>
      <div class="row between"><span>Spot USDC</span><b id="hl-spot">–</b></div>
      <div class="row between"><span>Perps (margin for agents)</span><b id="hl-perps">–</b></div>
      <div class="row end"><button class="secondary small-btn" id="hl-move">Move to perps</button></div>
    </section>`;

/** What each EVM chain holds, for the Send form. */
type Held = { symbol: string; token: string; decimals: number; balance: string };
const holdings: Record<number, Held[]> = {};
let evmAddr = '';

const toUnits = (amount: string, decimals: number) => {
  const [w, f = ''] = amount.trim().split('.');
  if (!/^\d+$/.test(w || '0') || !/^\d*$/.test(f) || f.length > decimals) throw new Error('Enter an amount like 1.5');
  return (BigInt(w || '0') * 10n ** BigInt(decimals) + BigInt((f + '0'.repeat(decimals)).slice(0, decimals) || '0')).toString();
};

let solana = { address: '', balance: '0' };
let tron = { address: '', balance: '0' };
const TRON_API = 'https://nile.trongrid.io';
let xrp = { address: '', balance: '0', key: '' };
// XRPL Labs' public testnet node: HTTPS on 443, with CORS.
const XRPL = 'https://testnet.xrpl-labs.com';

/** Sends XRP. The Payment is built on this page; the node only supplies the
 *  account's sequence, the current ledger and the fee (capped in core). */
function xrpSendForm() {
  const m = document.getElementById('modal')!;
  m.innerHTML = `
    <div class="backdrop"><form class="card sheet" id="send">
      <h2>Send on XRP</h2>
      <p class="small muted">Balance ${esc(xrp.balance)} XRP</p>
      <label>To <input name="to" required class="mono" placeholder="r…"></label>
      <label>Amount (XRP) <input name="amount" required inputmode="decimal" placeholder="0.0"></label>
      <label>Destination tag (optional) <input name="tag" inputmode="numeric" placeholder="Exchanges often need one"></label>
      <div class="row end"><button type="button" class="secondary" id="cancel">Cancel</button><button class="primary">Review</button></div>
      <div id="status" class="status"></div>
    </form></div>`;
  document.getElementById('cancel')!.onclick = () => (m.innerHTML = '');
  (document.getElementById('send') as HTMLFormElement).onsubmit = async (ev) => {
    ev.preventDefault();
    try {
      const f = new FormData(ev.target as HTMLFormElement);
      const to = String(f.get('to')).trim();
      const drops = toUnits(String(f.get('amount')), 6);
      const tagText = String(f.get('tag') ?? '').trim();
      if (tagText && !/^\d+$/.test(tagText)) throw new Error('The destination tag is a whole number');
      status('Preparing…');
      const [info, fee] = await Promise.all([
        post(XRPL, { method: 'account_info', params: [{ account: xrp.address, ledger_index: 'current' }] }),
        post(XRPL, { method: 'fee', params: [{}] }),
      ]);
      const a = info.result;
      if (a.error) throw new Error(a.error_message ?? a.error);
      const drop = BigInt(fee.result.drops.open_ledger_fee ?? '12');
      // About five minutes to approve and submit, then the payment expires.
      const last = Number(fee.result.ledger_current_index) + 75;
      const p = JSON.parse(core.xrp_payment(xrp.address, xrp.key, to, drops, String(drop < 12n ? 12n : drop), a.account_data.Sequence, last, tagText ? Number(tagText) : -1));
      const msg = core.owner_ecdsa_multi_sign_message(cfg.chain_id, wallet, nonce32(), createdAt(), cfg.signer, 'xrp', JSON.stringify([p.signing_hash]));
      confirm(msg, {
        xrpTx: p.tx_hex,
        after: async (out) => {
          if (!out.mpc_signature) throw new Error('No signature came back');
          const signed = JSON.parse(core.xrp_signed_tx(p.tx_hex, JSON.stringify(out.mpc_signature), xrp.address));
          const r = (await post(XRPL, { method: 'submit', params: [{ tx_blob: signed.tx_blob }] })).result;
          if (!['tesSUCCESS', 'terQUEUED'].includes(r.engine_result)) throw new Error(r.engine_result_message ?? r.error_message ?? r.engine_result ?? 'XRP refused the transaction');
          return `Sent. <a target="_blank" rel="noopener" href="https://testnet.xrpl.org/transactions/${esc(signed.hash)}">View transaction</a>`;
        },
      });
    } catch (e) {
      status(String((e as Error).message ?? e), 'error');
    }
  };
}

let ton = { address: '', balance: '0', key: '' };
let stellar = { address: '', balance: '0', key: '' };

/** Sends XLM. The transaction is built on this page (core); Horizon only
 *  supplies the sequence number, whether the recipient exists (if not, the
 *  send creates it) and the base fee (capped in core). */
function stellarSendForm() {
  const m = document.getElementById('modal')!;
  m.innerHTML = `
    <div class="backdrop"><form class="card sheet" id="send">
      <h2>Send on Stellar</h2>
      <p class="small muted">Balance ${esc(stellar.balance)} XLM</p>
      <label>To <input name="to" required class="mono" placeholder="G…"></label>
      <label>Amount (XLM) <input name="amount" required inputmode="decimal" placeholder="0.0"></label>
      <label>Memo (optional) <input name="memo" maxlength="28" placeholder="Exchanges often need one"></label>
      <div class="row end"><button type="button" class="secondary" id="cancel">Cancel</button><button class="primary">Review</button></div>
      <div id="status" class="status"></div>
    </form></div>`;
  document.getElementById('cancel')!.onclick = () => (m.innerHTML = '');
  (document.getElementById('send') as HTMLFormElement).onsubmit = async (ev) => {
    ev.preventDefault();
    try {
      const f = new FormData(ev.target as HTMLFormElement);
      const to = String(f.get('to')).trim();
      const stroops = toUnits(String(f.get('amount')), 7);
      const memo = String(f.get('memo') ?? '').trim();
      status('Preparing…');
      const [me, dest, fees] = await Promise.all([
        fetch(`${HORIZON}/accounts/${stellar.address}`).then((r) => (r.ok ? r.json() : null)),
        fetch(`${HORIZON}/accounts/${encodeURIComponent(to)}`).then((r) => r.status),
        get(`${HORIZON}/fee_stats`),
      ]);
      if (!me) throw new Error('This Stellar account has no XLM yet');
      const fee = Math.max(100, Number(fees.last_ledger_base_fee ?? 100));
      // Ten minutes to approve and submit, then it expires.
      const p = JSON.parse(core.stellar_payment(stellar.key, to, stroops, dest === 404, fee, (BigInt(me.sequence) + 1n).toString(), Math.floor(Date.now() / 1000) + 600, memo));
      const msg = core.owner_eddsa_sign_message(cfg.chain_id, wallet, nonce32(), createdAt(), cfg.signer, 'stellar', p.hash);
      confirm(msg, {
        stellarTx: p.tx_xdr,
        after: async (out) => {
          if (!out.mpc_signature) throw new Error('No signature came back');
          const env = core.stellar_signed_tx(p.tx_xdr, JSON.stringify(out.mpc_signature), stellar.key);
          const r = await fetch(`${HORIZON}/transactions`, { method: 'POST', headers: { 'content-type': 'application/x-www-form-urlencoded' }, body: `tx=${encodeURIComponent(env)}` });
          const j = await r.json();
          if (!r.ok) throw new Error(j.extras?.result_codes ? JSON.stringify(j.extras.result_codes) : (j.title ?? 'Stellar refused the transaction'));
          return `Sent. <a target="_blank" rel="noopener" href="https://stellar.expert/explorer/testnet/tx/${esc(j.hash)}">View transaction</a>`;
        },
      });
    } catch (e) {
      status(String((e as Error).message ?? e), 'error');
    }
  };
}

/** Sends GRAM (TON's coin, Toncoin until June 2026). The wallet message is built on this page (core); the node only
 *  supplies the wallet's seqno and whether it is deployed yet. The first
 *  send deploys the wallet (the address is its code and key's hash). */
function tonSendForm() {
  const m = document.getElementById('modal')!;
  m.innerHTML = `
    <div class="backdrop"><form class="card sheet" id="send">
      <h2>Send on TON</h2>
      <p class="small muted">Balance ${esc(ton.balance)} GRAM</p>
      <label>To <input name="to" required class="mono" placeholder="UQ… or EQ…"></label>
      <label>Amount (GRAM) <input name="amount" required inputmode="decimal" placeholder="0.0"></label>
      <label>Comment (optional) <input name="comment" maxlength="120" placeholder="Exchanges often need one"></label>
      <div class="row end"><button type="button" class="secondary" id="cancel">Cancel</button><button class="primary">Review</button></div>
      <div id="status" class="status"></div>
    </form></div>`;
  document.getElementById('cancel')!.onclick = () => (m.innerHTML = '');
  (document.getElementById('send') as HTMLFormElement).onsubmit = async (ev) => {
    ev.preventDefault();
    try {
      const f = new FormData(ev.target as HTMLFormElement);
      const to = String(f.get('to')).trim();
      const nano = toUnits(String(f.get('amount')), 9);
      const note = String(f.get('comment') ?? '').trim();
      status('Preparing…');
      const info = (await get(`${TONCENTER}/getWalletInformation?address=${encodeURIComponent(ton.address)}`)).result;
      const deployed = info.account_state === 'active';
      const seqno = deployed ? Number(info.seqno ?? 0) : 0;
      // Ten minutes to approve and send, then the message expires.
      const p = JSON.parse(core.ton_transfer(ton.key, to, nano, seqno, Math.floor(Date.now() / 1000) + 600, note));
      const msg = core.owner_eddsa_sign_message(cfg.chain_id, wallet, nonce32(), createdAt(), cfg.signer, 'ton', p.signing_hash);
      confirm(msg, {
        tonBoc: p.signing_boc,
        after: async (out) => {
          if (!out.mpc_signature) throw new Error('No signature came back');
          const boc = core.ton_signed_tx(p.signing_boc, JSON.stringify(out.mpc_signature), ton.key, !deployed);
          const r = await post(`${TONCENTER}/sendBoc`, { boc });
          if (!r.ok) throw new Error(r.error ?? 'TON refused the message');
          return `Sent. It shows on the explorer in a few seconds. <a target="_blank" rel="noopener" href="https://testnet.tonviewer.com/${esc(ton.address)}">View account</a>`;
        },
      });
    } catch (e) {
      status(String((e as Error).message ?? e), 'error');
    }
  };
}

/** Sends TRX. The transaction is built on this page from the latest block's
 *  reference, never by a node, so the passkey approves bytes the page made. */
function tronSendForm() {
  const m = document.getElementById('modal')!;
  m.innerHTML = `
    <div class="backdrop"><form class="card sheet" id="send">
      <h2>Send on TRON</h2>
      <p class="small muted">Balance ${esc(tron.balance)} TRX</p>
      <label>To <input name="to" required class="mono" placeholder="T…"></label>
      <label>Amount (TRX) <input name="amount" required inputmode="decimal" placeholder="0.0"></label>
      <div class="row end"><button type="button" class="secondary" id="cancel">Cancel</button><button class="primary">Review</button></div>
      <div id="status" class="status"></div>
    </form></div>`;
  document.getElementById('cancel')!.onclick = () => (m.innerHTML = '');
  (document.getElementById('send') as HTMLFormElement).onsubmit = async (ev) => {
    ev.preventDefault();
    try {
      const f = new FormData(ev.target as HTMLFormElement);
      const to = String(f.get('to')).trim();
      const sun = toUnits(String(f.get('amount')), 6);
      status('Preparing…');
      const b = await post(`${TRON_API}/wallet/getnowblock`, {});
      const h = b.block_header.raw_data;
      const refBytes = (BigInt(h.number) & 0xffffn).toString(16).padStart(4, '0');
      const refHash = b.blockID.slice(16, 32);
      // Ten minutes to approve and broadcast.
      const r = JSON.parse(core.tron_transfer_raw(tron.address, to, sun, refBytes, refHash, BigInt(h.timestamp + 600_000), BigInt(Date.now())));
      const msg = core.owner_ecdsa_multi_sign_message(cfg.chain_id, wallet, nonce32(), createdAt(), cfg.signer, 'tron', JSON.stringify([r.txid]));
      confirm(msg, {
        tronRaw: r.raw_hex,
        after: async (out) => {
          if (!out.mpc_signature) throw new Error('No signature came back');
          const signed = JSON.parse(core.tron_signed_tx(r.raw_hex, JSON.stringify(out.mpc_signature), tron.address));
          const res = await post(`${TRON_API}/wallet/broadcasthex`, { transaction: signed.tx_hex });
          if (!res.result) throw new Error(res.message ?? res.code ?? 'TRON refused the transaction');
          return `Sent. <a target="_blank" rel="noopener" href="https://nile.tronscan.org/#/transaction/${esc(signed.txid)}">View transaction</a>`;
        },
      });
    } catch (e) {
      status(String((e as Error).message ?? e), 'error');
    }
  };
}
const MEMPOOL = 'https://mempool.space/testnet4/api';

/** The Bitcoin-style chains with SegWit sends: the same plan and
 *  transaction format, each with its own explorer API (mempool.space's). */
const UTXO_SEND: Record<string, { name: string; sym: string; api: string; tx: string; placeholder: string }> = {
  bitcoin: { name: 'Bitcoin', sym: 'BTC', api: MEMPOOL, tx: 'https://mempool.space/testnet4/tx/', placeholder: 'tb1…' },
  litecoin: { name: 'Litecoin', sym: 'LTC', api: 'https://litecoinspace.org/testnet/api', tx: 'https://litecoinspace.org/testnet/tx/', placeholder: 'tltc1…' },
};
const utxoAccounts: Record<string, { address: string; balance: string }> = {};

/** Sends BTC or LTC: the page picks coins and builds the transaction, the
 *  passkey approves one signature per input, the page broadcasts once every
 *  signature checks out. */
function btcSendForm(chain = 'bitcoin') {
  const c = UTXO_SEND[chain];
  const acct = utxoAccounts[chain];
  const m = document.getElementById('modal')!;
  m.innerHTML = `
    <div class="backdrop"><form class="card sheet" id="send">
      <h2>Send on ${esc(c.name)}</h2>
      <p class="small muted">Balance ${esc(acct.balance)} ${c.sym}</p>
      <label>To <input name="to" required class="mono" placeholder="${c.placeholder}"></label>
      <label>Amount (${c.sym}) <input name="amount" required inputmode="decimal" placeholder="0.0"></label>
      <div class="row end"><button type="button" class="secondary" id="cancel">Cancel</button><button class="primary">Review</button></div>
      <div id="status" class="status"></div>
    </form></div>`;
  document.getElementById('cancel')!.onclick = () => (m.innerHTML = '');
  (document.getElementById('send') as HTMLFormElement).onsubmit = async (ev) => {
    ev.preventDefault();
    try {
      const f = new FormData(ev.target as HTMLFormElement);
      const to = String(f.get('to')).trim();
      const sat = toUnits(String(f.get('amount')), 8);
      status('Preparing…');
      const [utxos, fees] = await Promise.all([utxoGet(chain, `/address/${acct.address}/utxo`), utxoGet(chain, '/fees')]);
      const btcPlan = core.bitcoin_plan(acct.address, to, sat, JSON.stringify(utxos.map((u: any) => ({ txid: u.txid, vout: u.vout, value: u.value }))), Math.max(1, fees.halfHourFee));
      const msg = core.owner_ecdsa_multi_sign_message(cfg.chain_id, wallet, nonce32(), createdAt(), cfg.signer, chain, JSON.stringify(JSON.parse(btcPlan).sighashes));
      confirm(msg, {
        btcPlan,
        after: async (out) => {
          const sigs = (out as any).mpc_signatures ?? [];
          const signed = JSON.parse(core.bitcoin_signed_tx(btcPlan, JSON.stringify(sigs)));
          const txid = await utxoBroadcast(chain, signed.raw);
          return `Sent. <a target="_blank" rel="noopener" href="${c.tx}${esc(txid)}">View transaction</a>`;
        },
      });
    } catch (e) {
      status(String((e as Error).message ?? e), 'error');
    }
  };
}
/** Dogecoin, Bitcoin Cash and Zcash (transparent): P2PKH sends, read and
 *  broadcast through the relayer. `rate` is the fee per byte (Zcash's fee
 *  is ZIP-317's, set in core). */
const LEGACY_SEND: Record<string, { name: string; sym: string; tx: string; placeholder: string; rate: number }> = {
  dogecoin: { name: 'Dogecoin', sym: 'DOGE', tx: 'https://doge-testnet-explorer.qed.me/tx/{}', placeholder: 'n…', rate: 1000 },
  'bitcoin-cash': { name: 'Bitcoin Cash', sym: 'BCH', tx: 'https://explorer.selene.cash/tx/{}?network=chipnet', placeholder: 'bchtest:q…', rate: 2 },
  zcash: { name: 'Zcash', sym: 'ZEC', tx: 'https://testnet.zcashexplorer.app/transactions/{}', placeholder: 'tm…', rate: 1 },
};
const legacyAccounts: Record<string, { address: string; balance: string }> = {};
let ada = { address: '', balance: '0', key: '' };

async function relayerBroadcast(chain: string, raw: string): Promise<string> {
  const r = await fetch(`${RELAYER}/chain/${chain}/tx`, { method: 'POST', body: raw, signal: AbortSignal.timeout(30000) });
  const j = await r.json();
  if (!r.ok) throw new Error(j.error ?? 'broadcast refused');
  return j.txid;
}

/** A send sheet: To and Amount, then `review` builds what Confirm shows. */
function simpleSendForm(title: string, balance: string, sym: string, placeholder: string, review: (to: string, amount: string) => Promise<void>) {
  const m = document.getElementById('modal')!;
  m.innerHTML = `
    <div class="backdrop"><form class="card sheet" id="send">
      <h2>Send on ${esc(title)}</h2>
      <p class="small muted">Balance ${esc(balance)} ${esc(sym)}</p>
      <label>To <input name="to" required class="mono" placeholder="${esc(placeholder)}"></label>
      <label>Amount (${esc(sym)}) <input name="amount" required inputmode="decimal" placeholder="0.0"></label>
      <div class="row end"><button type="button" class="secondary" id="cancel">Cancel</button><button class="primary">Review</button></div>
      <div id="status" class="status"></div>
    </form></div>`;
  document.getElementById('cancel')!.onclick = () => (m.innerHTML = '');
  (document.getElementById('send') as HTMLFormElement).onsubmit = async (ev) => {
    ev.preventDefault();
    try {
      const f = new FormData(ev.target as HTMLFormElement);
      status('Preparing…');
      await review(String(f.get('to')).trim(), String(f.get('amount')));
    } catch (e) {
      status(String((e as Error).message ?? e), 'error');
    }
  };
}

function legacySendForm(chain: string) {
  const c = LEGACY_SEND[chain];
  const acct = legacyAccounts[chain];
  simpleSendForm(c.name, acct.balance, c.sym, c.placeholder, async (to, amount) => {
    const sat = toUnits(amount, 8);
    const enc = encodeURIComponent(acct.address);
    const [utxos, info] = await Promise.all([get(`${RELAYER}/chain/${chain}/utxos/${enc}`), chain === 'zcash' ? get(`${RELAYER}/chain/zcash/info`) : null]);
    if (!Array.isArray(utxos)) throw new Error(utxos?.error ?? `Couldn't read your ${c.sym} coins`);
    // Zcash: the signatures commit to the current branch and expire 40
    // blocks (about 50 minutes) from now.
    const legacyPlan = core.legacy_plan(chain, acct.address, to, sat, JSON.stringify(utxos), c.rate, info?.branch_id ?? 0, info ? info.height + 40 : 0);
    const msg = core.owner_ecdsa_multi_sign_message(cfg.chain_id, wallet, nonce32(), createdAt(), cfg.signer, chain, JSON.stringify(JSON.parse(legacyPlan).sighashes));
    confirm(msg, {
      legacyPlan,
      after: async (out) => {
        const signed = JSON.parse(core.legacy_signed_tx(legacyPlan, JSON.stringify((out as any).mpc_signatures ?? [])));
        await relayerBroadcast(chain, signed.raw);
        return `Sent. <a target="_blank" rel="noopener" href="${esc(c.tx.replace('{}', signed.txid))}">View transaction</a>`;
      },
    });
  });
}

/** Sends ADA: the page builds the transaction body, the passkey approves a
 *  signature over its hash, the page submits once the signature verifies. */
function adaSendForm() {
  simpleSendForm('Cardano', ada.balance, 'ADA', 'addr_test1…', async (to, amount) => {
    const lovelace = toUnits(amount, 6);
    const enc = encodeURIComponent(ada.address);
    const [utxos, tip] = await Promise.all([get(`${RELAYER}/chain/cardano/utxos/${enc}`), get(`${RELAYER}/chain/cardano/info`)]);
    if (!Array.isArray(utxos)) throw new Error(utxos?.error ?? "Couldn't read your ADA");
    // Valid for two hours of slots (one a second).
    const adaPlan = core.cardano_plan(ada.address, to, lovelace, JSON.stringify(utxos), BigInt(tip.slot + 7200));
    const msg = core.owner_eddsa_sign_message(cfg.chain_id, wallet, nonce32(), createdAt(), cfg.signer, 'cardano', JSON.parse(adaPlan).hash);
    confirm(msg, {
      adaPlan,
      after: async (out) => {
        if (!out.mpc_signature) throw new Error('No signature came back');
        const signed = JSON.parse(core.cardano_signed_tx(adaPlan, JSON.stringify(out.mpc_signature), ada.key));
        await relayerBroadcast('cardano', signed.cbor);
        return `Sent. <a target="_blank" rel="noopener" href="https://preprod.cardanoscan.io/transaction/${esc(signed.txid)}">View transaction</a>`;
      },
    });
  });
}
const SOLANA_RPC = 'https://api.devnet.solana.com';

/** Sends SOL: the passkey approves the exact message, the account's ed25519
 *  key signs it, and the page broadcasts it once the signature verifies. */
function solSendForm() {
  const m = document.getElementById('modal')!;
  m.innerHTML = `
    <div class="backdrop"><form class="card sheet" id="send">
      <h2>Send on Solana</h2>
      <p class="small muted">Balance ${esc(solana.balance)} SOL</p>
      <label>To <input name="to" required class="mono" placeholder="Solana address"></label>
      <label>Amount (SOL) <input name="amount" required inputmode="decimal" placeholder="0.0"></label>
      <div class="row end"><button type="button" class="secondary" id="cancel">Cancel</button><button class="primary">Review</button></div>
      <div id="status" class="status"></div>
    </form></div>`;
  document.getElementById('cancel')!.onclick = () => (m.innerHTML = '');
  (document.getElementById('send') as HTMLFormElement).onsubmit = async (ev) => {
    ev.preventDefault();
    try {
      const f = new FormData(ev.target as HTMLFormElement);
      const to = String(f.get('to')).trim();
      const lamports = toUnits(String(f.get('amount')), 9);
      status('Preparing…');
      const bh = (await post(SOLANA_RPC, { jsonrpc: '2.0', id: 1, method: 'getLatestBlockhash', params: [{ commitment: 'finalized' }] })).result.value.blockhash;
      const solMessage = core.solana_transfer_message(solana.address, to, lamports, bh);
      const msg = core.owner_eddsa_sign_message(cfg.chain_id, wallet, nonce32(), createdAt(), cfg.signer, 'solana', solMessage);
      confirm(msg, {
        solMessage,
        after: async (out) => {
          if (!out.mpc_signature) throw new Error('No signature came back');
          const signed = core.solana_signed_tx(solMessage, JSON.stringify(out.mpc_signature), solana.address);
          const r = await post(SOLANA_RPC, { jsonrpc: '2.0', id: 1, method: 'sendTransaction', params: [signed, { encoding: 'base64' }] });
          if (r.error) throw new Error(r.error.message);
          return `Sent. <a target="_blank" rel="noopener" href="https://explorer.solana.com/tx/${esc(r.result)}?cluster=devnet">View transaction</a>`;
        },
      });
    } catch (e) {
      status(String((e as Error).message ?? e), 'error');
    }
  };
}

/** Sends from the account's EVM address on one chain: the passkey approves
 *  the exact transaction, then the page broadcasts it. */
function sendForm(chainId: number) {
  const c = EVM_CHAINS.find((x) => x.id === chainId)!;
  const assets = holdings[chainId] ?? [];
  const m = document.getElementById('modal')!;
  if (!assets.length) {
    m.innerHTML = `
      <div class="backdrop"><div class="card sheet">
        <h2>Send on ${esc(c.name)}</h2>
        <p class="muted">Nothing to send on ${esc(c.name)} yet. Use Deposit to add funds first.</p>
        <div class="row end"><button class="secondary" id="cancel">Close</button></div>
      </div></div>`;
    document.getElementById('cancel')!.onclick = () => (m.innerHTML = '');
    return;
  }
  m.innerHTML = `
    <div class="backdrop"><form class="card sheet" id="send">
      <h2>Send on ${esc(c.name)}</h2>
      <label>Asset <select name="asset">${assets.map((a, i) => `<option value="${i}">${esc(a.symbol)} (${esc(a.balance)})</option>`).join('')}</select></label>
      <label>To <input name="to" required pattern="0x[0-9a-fA-F]{40}" class="mono" placeholder="0x…"></label>
      <label>Amount <input name="amount" required inputmode="decimal" placeholder="0.0"></label>
      <div class="row end"><button type="button" class="secondary" id="cancel">Cancel</button><button class="primary">Review</button></div>
      <div id="status" class="status"></div>
    </form></div>`;
  document.getElementById('cancel')!.onclick = () => (m.innerHTML = '');
  (document.getElementById('send') as HTMLFormElement).onsubmit = async (ev) => {
    ev.preventDefault();
    try {
      const f = new FormData(ev.target as HTMLFormElement);
      const a = assets[Number(f.get('asset'))];
      const to = String(f.get('to')).trim();
      const amount = toUnits(String(f.get('amount')), a.decimals);
      status('Preparing…');
      const native = !a.token;
      const data = native ? '0x' : '0xa9059cbb' + to.slice(2).toLowerCase().padStart(64, '0') + BigInt(amount).toString(16).padStart(64, '0');
      const [nonce, block, prio, gas] = await Promise.all([
        evmRpc(c.rpc, 'eth_getTransactionCount', [evmAddr, 'pending']),
        fetch(c.rpc, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'eth_getBlockByNumber', params: ['latest', false] }) }).then((r) => r.json()),
        evmRpc(c.rpc, 'eth_maxPriorityFeePerGas', []).catch(() => 1_000_000n),
        evmRpc(c.rpc, 'eth_estimateGas', [{ from: evmAddr, to: native ? to : a.token, value: native ? '0x' + BigInt(amount).toString(16) : '0x0', data }]),
      ]);
      const maxFee = BigInt(block.result.baseFeePerGas) * 2n + prio;
      const tx = core.evm_send_tx(BigInt(chainId), nonce, prio.toString(), maxFee.toString(), (gas * 12n) / 10n, a.token, to, amount);
      const msg = core.owner_evm_sign_message(cfg.chain_id, wallet, nonce32(), createdAt(), cfg.signer, tx);
      confirm(msg, {
        tx,
        after: async (out) => {
          if (!out.mpc_signature) throw new Error('No signature came back');
          const signed = JSON.parse(core.evm_signed_tx(tx, JSON.stringify(out.mpc_signature)));
          // Broadcast only what this account signed.
          if (signed.from.toLowerCase() !== evmAddr.toLowerCase()) throw new Error('The signature is not from this account');
          const r = await fetch(c.rpc, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'eth_sendRawTransaction', params: [signed.raw] }) }).then((r) => r.json());
          if (r.error) throw new Error(r.error.message);
          return c.explorer ? `Sent. <a target="_blank" rel="noopener" href="${esc(c.explorer.replace('/address/', '/tx/') + r.result)}">View transaction</a>` : `Sent: ${esc(r.result)}`;
        },
      });
    } catch (e) {
      status(String((e as Error).message ?? e), 'error');
    }
  };
}

/** Offers to move a wallet still on an older Marvii extension to the
 *  current one (adds it, removes the old: one passkey tap). */
async function moveCard() {
  const have = await view<string[]>(wallet, 'w_extensions', {});
  if (have.includes(cfg.extension)) return;
  const old = have.filter((e) => (cfg.known_extensions ?? []).includes(e));
  if (!old.length) return;
  document.getElementById('move-card')!.hidden = false;
  document.getElementById('move')!.onclick = () => confirm(core.extension_move_message(cfg.chain_id, wallet, nonce(), createdAt(), cfg.extension, JSON.stringify(old)));
}

const HL_API = 'https://api.hyperliquid-testnet.xyz';
const hlInfo = (type: string, user: string) => post(`${HL_API}/info`, { type, user });

/** The account's Hyperliquid balances (same address as on every EVM
 *  chain), and moving USDC from spot to perps, where agents' orders draw
 *  margin. Money sent to the account on Hyperliquid lands in spot. */
async function hlCard() {
  const addr = await evmAddress();
  const [spot, perps] = await Promise.all([hlInfo('spotClearinghouseState', addr), hlInfo('clearinghouseState', addr)]);
  const usdc = spot.balances?.find((b: any) => b.coin === 'USDC');
  const spotTotal = usdc ? Number(usdc.total) - Number(usdc.hold) : 0;
  const card = document.getElementById('hl-card');
  if (!card) return;
  if (!usdc && Number(perps.marginSummary?.accountValue ?? 0) === 0) return; // nothing on Hyperliquid
  card.hidden = false;
  document.getElementById('hl-spot')!.textContent = `${spotTotal.toFixed(2)} USDC`;
  document.getElementById('hl-perps')!.textContent = `${Number(perps.marginSummary?.accountValue ?? 0).toFixed(2)} USDC`;
  const btn = document.getElementById('hl-move') as HTMLButtonElement;
  btn.disabled = spotTotal <= 0 || document.body.classList.contains('readonly');
  btn.onclick = () => {
    const m = document.getElementById('modal')!;
    m.innerHTML = `
      <div class="backdrop"><form class="card sheet" id="send">
        <h2>Move to perps</h2>
        <p class="small muted">Spot ${esc(spotTotal.toFixed(2))} USDC. Agents trade perps with this as margin, within the limits you gave them.</p>
        <label>Amount (USDC) <input name="amount" required inputmode="decimal" value="${esc(String(Math.floor(spotTotal)))}"></label>
        <div class="row end"><button type="button" class="secondary" id="cancel">Cancel</button><button class="primary">Review</button></div>
        <div id="status" class="status"></div>
      </form></div>`;
    document.getElementById('cancel')!.onclick = () => (m.innerHTML = '');
    (document.getElementById('send') as HTMLFormElement).onsubmit = (ev) => {
      ev.preventDefault();
      try {
        // Hyperliquid's amount string: plain digits, no trailing zeros.
        const amount = String(Number(String(new FormData(ev.target as HTMLFormElement).get('amount')).trim()));
        const nonce = Date.now();
        const msg = core.owner_hl_transfer_message(cfg.chain_id, wallet, nonce32(), createdAt(), cfg.signer, amount, true, BigInt(nonce));
        confirm(msg, {
          hlTransfer: { amount, to_perp: true, nonce },
          after: async (out) => {
            if (!out.mpc_signature) throw new Error('No signature came back');
            const sig = out.mpc_signature as any;
            const N = 0xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141n;
            let sv = BigInt('0x' + sig.s.scalar);
            let v = sig.recovery_id;
            if (sv > N / 2n) { sv = N - sv; v ^= 1; }
            const signature = { r: '0x' + sig.big_r.affine_point.slice(2), s: '0x' + sv.toString(16).padStart(64, '0'), v: 27 + v };
            const action = JSON.parse(core.hl_transfer_action(amount, true, BigInt(nonce)));
            const r = await post(`${HL_API}/exchange`, { action, nonce, signature });
            if (r.status !== 'ok') throw new Error(`Hyperliquid: ${typeof r.response === 'string' ? r.response : JSON.stringify(r)}`);
            return `Moved ${esc(amount)} USDC to perps on Hyperliquid testnet.`;
          },
        });
      } catch (e) {
        status(String((e as Error).message ?? e), 'error');
      }
    };
  };
}

type IntentsToken = { asset: string; symbol: string; chain: string; decimals: number; price: number };

/** The Swap card: a dry NEAR Intents quote (1Click, through the relayer,
 *  which always asks with `dry: true`). Nothing is deposited or signed. */
function swapCard() {
  const form = document.getElementById('swap') as HTMLFormElement | null;
  if (!form) return;
  const out = document.getElementById('swap-out')!;
  const names: Record<string, string> = { near: 'NEAR', eth: 'Ethereum', btc: 'Bitcoin', sol: 'Solana', zec: 'Zcash', doge: 'Dogecoin', xrp: 'XRP', tron: 'TRON', cardano: 'Cardano', ltc: 'Litecoin', bch: 'Bitcoin Cash' };
  let tokens: IntentsToken[] = [];
  get(`${RELAYER}/intents/tokens`)
    .then((t: IntentsToken[]) => {
      tokens = t;
      const opts = t.map((x) => `<option value="${esc(x.asset)}">${esc(x.symbol)} · ${esc(names[x.chain] ?? x.chain)}</option>`).join('');
      for (const n of ['from', 'to']) (form.elements.namedItem(n) as HTMLSelectElement).innerHTML = opts;
      (form.elements.namedItem('to') as HTMLSelectElement).selectedIndex = Math.min(1, t.length - 1);
    })
    .catch(() => (out.innerHTML = '<span class="muted">Quotes are unavailable right now.</span>'));
  form.onsubmit = async (ev) => {
    ev.preventDefault();
    const f = new FormData(form);
    const from = tokens.find((x) => x.asset === f.get('from'));
    const to = tokens.find((x) => x.asset === f.get('to'));
    if (!from || !to) return;
    try {
      if (from.asset === to.asset) throw new Error('Pick two different assets');
      out.innerHTML = '<span class="muted">Asking NEAR Intents…</span>';
      const r = await fetch(`${RELAYER}/intents/quote`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ from: from.asset, to: to.asset, amount: toUnits(String(f.get('amount')), from.decimals), account: wallet }),
        signal: AbortSignal.timeout(25000),
      });
      const j = await r.json();
      if (!r.ok) throw new Error(/liquidity/i.test(j.error ?? '') ? 'No solver is quoting this pair right now. Try another.' : j.error ?? 'No quote');
      const q = j.quote;
      const usd = (v: string) => (v ? ` ($${Number(v).toFixed(2)})` : '');
      out.innerHTML = `<div class="row between"><span>You'd get</span><b>${esc(q.amountOutFormatted)} ${esc(to.symbol)}${esc(usd(q.amountOutUsd))}</b></div>
        <div class="row between muted"><span>At least</span><span>${esc(fmtUnits(BigInt(q.minAmountOut), to.decimals, Math.min(8, to.decimals)))} ${esc(to.symbol)} (1% slippage)</span></div>
        <div class="row between muted"><span>For</span><span>${esc(q.amountInFormatted)} ${esc(from.symbol)}${esc(usd(q.amountInUsd))}</span></div>
        <div class="row between muted"><span>Time</span><span>about ${esc(q.timeEstimate)} s</span></div>`;
    } catch (e) {
      out.innerHTML = `<span class="error">${esc((e as Error).message ?? e)}</span>`;
    }
  };
}

/** Mainnet assets a top-up moves (1Click's /v0/tokens; the extension's
 *  intents.rs has the same table). `rpc` marks a chain the page can deposit
 *  from: the passkey approves the exact transfer to 1Click's deposit address. */
const TOPUP: { key: string; id: string; label: string; decimals: number; family: 'evm' | 'sol' | 'near'; chain?: number; token?: string; rpc?: string; explorer?: string }[] = [
  { key: 'base-usdc', id: 'nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near', label: 'USDC · Base', decimals: 6, family: 'evm', chain: 8453, token: '0x833589fcd6edb6e08f4c7c32d4f71b54bda02913', rpc: 'https://mainnet.base.org', explorer: 'https://basescan.org/tx/' },
  { key: 'base-eth', id: 'nep141:base.omft.near', label: 'ETH · Base', decimals: 18, family: 'evm', chain: 8453, rpc: 'https://mainnet.base.org', explorer: 'https://basescan.org/tx/' },
  { key: 'arb-usdc', id: 'nep141:arb-0xaf88d065e77c8cc2239327c5edb3a432268e5831.omft.near', label: 'USDC · Arbitrum', decimals: 6, family: 'evm', chain: 42161, token: '0xaf88d065e77c8cc2239327c5edb3a432268e5831', rpc: 'https://arb1.arbitrum.io/rpc', explorer: 'https://arbiscan.io/tx/' },
  { key: 'arb-eth', id: 'nep141:arb.omft.near', label: 'ETH · Arbitrum', decimals: 18, family: 'evm', chain: 42161, rpc: 'https://arb1.arbitrum.io/rpc', explorer: 'https://arbiscan.io/tx/' },
  { key: 'eth', id: 'nep141:eth.omft.near', label: 'ETH · Ethereum', decimals: 18, family: 'evm', chain: 1, rpc: 'https://ethereum-rpc.publicnode.com', explorer: 'https://etherscan.io/tx/' },
  { key: 'sol', id: 'nep141:sol.omft.near', label: 'SOL · Solana', decimals: 9, family: 'sol' },
  { key: 'sol-usdc', id: 'nep141:sol-5ce3bf3a31af18be40ba30f721101b4341690186.omft.near', label: 'USDC · Solana', decimals: 6, family: 'sol' },
  { key: 'near-usdc', id: 'nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1', label: 'USDC · NEAR', decimals: 6, family: 'near' },
  { key: 'wnear', id: 'nep141:wrap.near', label: 'wNEAR · NEAR', decimals: 24, family: 'near' },
];

function topupCard() {
  const form = document.getElementById('topup') as HTMLFormElement | null;
  if (!form) return;
  const out = document.getElementById('topup-out')!;
  // A testnet account has no NEAR account on mainnet to be paid.
  const dests = TOPUP.filter((a) => a.family !== 'near' || cfg.chain_id === 'mainnet');
  (form.elements.namedItem('from') as HTMLSelectElement).innerHTML = TOPUP.filter((a) => a.rpc).map((a) => `<option value="${a.key}">${esc(a.label)}</option>`).join('');
  (form.elements.namedItem('to') as HTMLSelectElement).innerHTML = dests.map((a) => `<option value="${a.key}">${esc(a.label)}</option>`).join('');
  (form.elements.namedItem('to') as HTMLSelectElement).value = 'sol';
  const own = (a: (typeof TOPUP)[number]) => (a.family === 'evm' ? evmAddr : a.family === 'sol' ? solana.address : wallet);
  form.onsubmit = async (ev) => {
    ev.preventDefault();
    const f = new FormData(form);
    const from = TOPUP.find((a) => a.key === f.get('from'))!;
    const to = TOPUP.find((a) => a.key === f.get('to'))!;
    try {
      if (from.id === to.id) throw new Error('Pick two different assets');
      if (!own(from) || !own(to)) throw new Error('Your addresses are still loading. Try again in a moment.');
      out.innerHTML = '<span class="muted">Asking NEAR Intents…</span>';
      const r = await fetch(`${RELAYER}/intents/deposit-quote`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ from: from.id, to: to.id, amount: toUnits(String(f.get('amount')), from.decimals), refund_to: own(from), recipient: own(to) }),
        signal: AbortSignal.timeout(30000),
      });
      const q = await r.json();
      if (!r.ok) throw new Error(/liquidity/i.test(q.error ?? '') ? 'No solver is quoting this pair right now. Try another.' : q.error ?? 'No quote');
      const usd = (v: string) => (v ? ` ($${Number(v).toFixed(2)})` : '');
      out.innerHTML = `<div class="row between"><span>You'd get</span><b>${esc(q.amount_out)} ${esc(to.label)}${esc(usd(q.amount_out_usd))}</b></div>
        <div class="row between muted"><span>For</span><span>${esc(fmtUnits(BigInt(q.amount_in), from.decimals, Math.min(8, from.decimals)))} ${esc(from.label)}${esc(usd(q.amount_in_usd))}</span></div>
        <div class="row between muted"><span>Paid to</span><span class="mono">${esc(short(own(to)))} (you)</span></div>
        <div class="row between muted"><span>Time</span><span>about ${esc(q.time_estimate_s)} s</span></div>
        <div class="row end"><button type="button" class="primary" id="topup-go">Deposit with passkey</button></div>
        <div id="topup-status"></div>`;
      document.getElementById('topup-go')!.onclick = () => topupDeposit(from, q).catch((e) => (document.getElementById('topup-status')!.innerHTML = `<span class="error">${esc((e as Error).message ?? e)}</span>`));
    } catch (e) {
      out.innerHTML = `<span class="error">${esc((e as Error).message ?? e)}</span>`;
    }
  };
}

/** The owner's deposit: the passkey approves the exact transfer of the quoted
 *  amount to 1Click's deposit address; the page broadcasts it, tells 1Click,
 *  and follows the swap. */
async function topupDeposit(from: (typeof TOPUP)[number], q: { deposit_address: string; amount_in: string }) {
  const st = document.getElementById('topup-status')!;
  const rpc = from.rpc!;
  const amount = q.amount_in;
  const native = !from.token;
  const data = native ? '0x' : '0xa9059cbb' + q.deposit_address.slice(2).toLowerCase().padStart(64, '0') + BigInt(amount).toString(16).padStart(64, '0');
  st.innerHTML = '<span class="muted">Preparing…</span>';
  const [nonce, block, prio, gas] = await Promise.all([
    evmRpc(rpc, 'eth_getTransactionCount', [evmAddr, 'pending']),
    fetch(rpc, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'eth_getBlockByNumber', params: ['latest', false] }) }).then((r) => r.json()),
    evmRpc(rpc, 'eth_maxPriorityFeePerGas', []).catch(() => 1_000_000n),
    evmRpc(rpc, 'eth_estimateGas', [{ from: evmAddr, to: native ? q.deposit_address : from.token, value: native ? '0x' + BigInt(amount).toString(16) : '0x0', data }]),
  ]);
  const maxFee = BigInt(block.result.baseFeePerGas) * 2n + prio;
  const tx = core.evm_send_tx(BigInt(from.chain!), nonce, prio.toString(), maxFee.toString(), (gas * 12n) / 10n, from.token ?? "", q.deposit_address, amount);
  const msg = core.owner_evm_sign_message(cfg.chain_id, wallet, nonce32(), createdAt(), cfg.signer, tx);
  st.innerHTML = '';
  confirm(msg, {
    tx,
    after: async (out) => {
      if (!out.mpc_signature) throw new Error('No signature came back');
      const signed = JSON.parse(core.evm_signed_tx(tx, JSON.stringify(out.mpc_signature)));
      if (signed.from.toLowerCase() !== evmAddr.toLowerCase()) throw new Error('The signature is not from this account');
      const r = await fetch(rpc, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'eth_sendRawTransaction', params: [signed.raw] }) }).then((r) => r.json());
      if (r.error) throw new Error(r.error.message);
      await post(`${RELAYER}/intents/submit`, { tx_hash: r.result, deposit_address: q.deposit_address }).catch(() => {});
      followTopup(q.deposit_address);
      return `Deposited. <a target="_blank" rel="noopener" href="${esc(from.explorer + r.result)}">View transaction</a>. NEAR Intents is swapping it; progress shows under the quote.`;
    },
  });
}

async function followTopup(depositAddress: string) {
  const st = document.getElementById('topup-status');
  for (let i = 0; i < 60 && st; i++) {
    await new Promise((r) => setTimeout(r, 10_000));
    const s = await get(`${RELAYER}/intents/status?deposit_address=${encodeURIComponent(depositAddress)}`).catch(() => null);
    if (!s) continue;
    const words: Record<string, string> = { PENDING_DEPOSIT: 'Waiting for the deposit', KNOWN_DEPOSIT_TX: 'Deposit seen', PROCESSING: 'Swapping', SUCCESS: 'Done', REFUNDED: 'Refunded to you', FAILED: 'Failed', INCOMPLETE_DEPOSIT: 'Deposit incomplete' };
    st.innerHTML = `<span class="${s.status === 'FAILED' ? 'error' : 'muted'}">${esc(words[s.status] ?? s.status)}${s.amount_out ? ` · ${esc(s.amount_out)}` : ''}</span>`;
    if (['SUCCESS', 'REFUNDED', 'FAILED'].includes(s.status)) break;
  }
}

async function dashboard() {
  const all: [string, string][] = [['chain-near', 'NEAR'], ...EVM_CHAINS.map((c) => [`chain-${c.id}`, c.name] as [string, string]), ...OTHER_CHAINS.map((c) => [`chain-${c.id}`, c.name] as [string, string])];
  render(`
    <div id="status" class="status notice"></div>
    <section class="view" data-view="home">
      <div class="card account-card">
        <div class="small muted">Your account · ${esc(cfg.chain_id === 'mainnet' ? 'Mainnet' : 'Testnet')}</div>
        <div class="acct-name" id="acct-name">${esc(short(wallet))}</div>
        <div class="small muted">One passkey, ${all.length} chains.</div>
      </div>
      <nav class="tiles3">
        <a class="tile" href="#/assets"><span class="tile-ic">${ICON_ASSETS}</span><b>Assets</b><span class="small muted">${all.length} chains</span></a>
        <div class="tile-wrap">
          <button type="button" class="tile" id="swap-tile" aria-haspopup="menu" aria-expanded="false" aria-controls="swap-menu"><span class="tile-ic">${ICON_SWAP}</span><b>Swap <span class="caret" aria-hidden="true">▾</span></b><span class="small muted">NEAR Intents</span></button>
          <div class="tile-menu" id="swap-menu" role="menu" hidden>
            <a role="menuitem" href="#/swap"><b>Swap</b><span class="small muted">Trade balances inside NEAR Intents</span></a>
            <a role="menuitem" href="#/topup"><b>Top up another chain</b><span class="small muted">Move your funds to your address on another chain</span></a>
          </div>
        </div>
        <a class="tile" href="#/agents"><span class="tile-ic">${ICON_AGENTS}</span><b>Apps &amp; agents</b><span class="small muted" id="agents-note">Limits you set</span></a>
      </nav>
      <a class="card alert" id="pending-alert" href="#/agents" hidden><b>Waiting for you</b><span class="small" id="pending-alert-text"></span></a>
      <section class="card" id="move-card" hidden>
        <h2>Update available</h2>
        <p class="small">A newer Marvii extension gives agents more venues (Uniswap, Aave, Hyperliquid) within limits you set. One tap moves your account to it; the address and funds stay the same. Agents added before need adding again.</p>
        <div class="row end"><button class="primary" id="move">Update with passkey</button></div>
      </section>
    </section>
    <section class="view" data-view="assets">
      <a class="back" href="#/">‹ Home</a>
      <div class="row between"><h1>Assets</h1><span class="pill">${esc(cfg.chain_id === 'mainnet' ? 'Mainnet' : 'Testnet')}</span></div>
      <div class="chain-grid">${all.map(([id, n]) => chainTile(id, n)).join('')}</div>
    </section>
    ${all.map(([id, n]) => chainView(id, n)).join('')}
    <section class="view" data-view="swap">
      <a class="back" href="#/">‹ Home</a>
      <section class="card" id="swap-card">
        <div class="row between"><h1>Swap</h1><span class="pill">NEAR Intents · mainnet quote</span></div>
        <form id="swap" class="stack">
          <div class="row"><label class="grow">From <select name="from"></select></label><label class="grow">To <select name="to"></select></label></div>
          <label>Amount <input name="amount" required inputmode="decimal" placeholder="0.0"></label>
          <div class="row end"><button class="primary">Get quote</button></div>
          <div id="swap-out" class="small"></div>
        </form>
        <p class="muted small">Live prices from NEAR Intents' solvers, between balances held in NEAR Intents by this account. Quote only: this swaps balances already inside NEAR Intents. To move funds between chains, use <a href="#/topup">Top up another chain</a>.</p>
      </section>
    </section>
    <section class="view" data-view="topup">
      <a class="back" href="#/">‹ Home</a>
      <section class="card" id="topup-card">
        <div class="row between"><h2>Top up another chain</h2><span class="pill">NEAR Intents · mainnet</span></div>
        <form id="topup" class="stack">
          <div class="row"><label class="grow">From <select name="from"></select></label><label class="grow">To <select name="to"></select></label></div>
          <label>Amount <input name="amount" required inputmode="decimal" placeholder="0.0"></label>
          <div class="row end"><button class="primary">Get quote</button></div>
          <div id="topup-out" class="small"></div>
        </form>
        <p class="muted small">Moves your own funds between chains: 1Click's solvers pay your own address on the other chain, and refund your own address if the swap can't happen. Agents can do this too, and the extension checks 1Click's signed quote before it signs. These are real mainnet funds: your EVM and Solana addresses are the same on mainnet.</p>
      </section>
    </section>
    <section class="view" data-view="agents">
      <a class="back" href="#/">‹ Home</a>
      <section class="card" id="pending-card" hidden>
        <h2>Waiting for you</h2>
        <p class="muted small">Requests outside an agent's limits. Nothing happens unless you approve.</p>
        <ul id="pending" class="list"></ul>
      </section>
      <section class="card" id="queued-card" hidden>
        <h2>Queued</h2>
        <p class="muted small">New recipients or contracts an agent asked for. Each runs only after its wait, unless you cancel it.</p>
        <ul id="queued" class="list"></ul>
      </section>
      <section class="card">
        <div class="row between wrap-row"><h1>Apps &amp; agents</h1>
          <div class="pick-wrap">
            <button type="button" class="pick" id="protocol-pick" aria-haspopup="menu" aria-expanded="false" aria-controls="protocol-menu"><span>By protocol…</span><span class="caret" aria-hidden="true">▾</span></button>
            <div class="pick-menu" id="protocol-menu" role="menu" hidden>${PROTOCOLS.map((p) => `<a role="menuitem" href="#/p-${p.id}"><span>${esc(p.name)}</span><span class="mono">${esc(p.chain)}</span></a>`).join('')}</div>
          </div></div>
        <p class="muted small">NEAR checks every request against the limits you set. Revoke any agent instantly.</p>
        <ul id="sessions" class="list agents"><li class="muted">Loading…</li></ul>
      </section>
      <section class="card">
        <h2>Connect an app</h2>
        <p class="muted small">Apps and agents ask for access with a permission link. Open it here and you'll see exactly what they ask for before your passkey approves anything.</p>
        <form class="row" id="connect-paste"><input name="link" class="mono" placeholder="https://near.marvii.xyz/#/connect?r=…" required><button class="secondary">Open</button></form>
        <details class="small"><summary>Building an agent?</summary>
          <p>Use <span class="mono">@replico-labs/marvii-sdk</span> (in <span class="mono">near/sdk</span>): make a session key inside Marvii's OutLayer module (TEE), send the owner a permission link, wait for their approval, then act. Every action is checked by Marvii's extension on NEAR before Chain Signatures sign it. This page approves only keys made inside that module; it checks the run on NEAR.</p>
          <p>Testing without the SDK? <button type="button" class="secondary small-btn" id="add">Add a key by hand</button></p>
          <pre class="mono small">const signer = new OutLayerSigner({ moduleUrl, moduleHash, operator });
const publicKey = await signer.getPublicKey();
const link = permissionLink({ publicKey, keyProof: signer.keyProof, keyId: 'my-agent',
  app: { name: 'My agent' },
  grant: { near_venues: [{ receiver_id: 'wrap.testnet', method: 'near_deposit' }],
           caps: [{ asset: 'near', daily: '50000000000000000000000' }] } });
const { wallet } = await waitForApproval({ publicKey });
await new MarviiAgent({ wallet, keyId: 'my-agent', signer }).execute(nearCall(…));</pre>
        </details>
      </section>
    </section>
    ${protocolViews()}
    <div id="modal"></div>`);
  route();
  menuButton('swap-tile', 'swap-menu');
  menuButton('protocol-pick', 'protocol-menu');
  document.getElementById('add')!.onclick = grantForm;
  (document.getElementById('connect-paste') as HTMLFormElement).onsubmit = (ev) => {
    ev.preventDefault();
    const raw = String(new FormData(ev.target as HTMLFormElement).get('link') ?? '').trim();
    const m = raw.match(/#\/connect\?r=[A-Za-z0-9_-]+$/);
    if (!m) return status('That isn\'t a Marvii permission link.', 'error');
    location.hash = m[0];
    connectFromLink().catch((e) => status(`That permission link can't be used: ${(e as Error).message ?? e}`, 'error'));
  };
  connectFromLink().catch((e) => status(`That permission link can't be used: ${(e as Error).message ?? e}`, 'error'));
  swapCard();
  topupCard();
  hlCard().catch(() => {});
  if (!document.body.classList.contains('readonly')) moveCard().catch(() => {});
  app.onclick = (e) => {
    const b = (e.target as HTMLElement).closest<HTMLElement>('[data-copy]');
    if (b) {
      navigator.clipboard?.writeText(b.dataset.copy!).catch(() => {});
      b.classList.add('copied');
      setTimeout(() => b.classList.remove('copied'), 1200);
    }
    if ((e.target as HTMLElement).closest('[data-name]')) nameForm();
    const dep = (e.target as HTMLElement).closest<HTMLElement>('[data-deposit]');
    if (dep) depositSheet(dep.dataset.deposit!);
    const sb = (e.target as HTMLElement).closest<HTMLElement>('[data-send]');
    if (sb) {
      const k = sb.dataset.send!;
      k === 'near' ? nearSendForm() : k === 'solana' ? solSendForm() : k === 'bitcoin' || k === 'litecoin' ? btcSendForm(k) : k === 'tron' ? tronSendForm() : k === 'xrp' ? xrpSendForm() : k === 'ton' ? tonSendForm() : k === 'stellar' ? stellarSendForm() : k === 'cardano' ? adaSendForm() : k in LEGACY_SEND ? legacySendForm(k) : sendForm(Number(k));
    }
  };

  fillNear().catch(() => fillChain('chain-near', wallet, '– NEAR', []));
  for (const c of OTHER_CHAINS) {
    const { path, domain_id } = JSON.parse(core.chain_path(c.id));
    view<string>(cfg.signer, 'derived_public_key', { path, predecessor: wallet, domain_id })
      .then(async (k) => {
        const addr = core.chain_address(c.id, k);
        const bal = c.balance ? await c.balance(addr).then((b) => fmtUnits(b, c.decimals)).catch(() => '–') : '–';
        const explorer = c.explorer ? (c.explorer.includes('{}') ? c.explorer.replace('{}', addr) : c.explorer + addr) : undefined;
        if (c.id === 'solana') solana = { address: addr, balance: bal };
        if (c.id in UTXO_SEND) utxoAccounts[c.id] = { address: addr, balance: bal };
        if (c.id === 'tron') tron = { address: addr, balance: bal };
        if (c.id === 'xrp') xrp = { address: addr, balance: bal, key: k };
        if (c.id === 'ton') ton = { address: addr, balance: bal, key: k };
        if (c.id === 'stellar') stellar = { address: addr, balance: bal, key: k };
        if (c.id in LEGACY_SEND) legacyAccounts[c.id] = { address: addr, balance: bal };
        if (c.id === 'cardano') ada = { address: addr, balance: bal, key: k };
        const canSend = ['solana', 'bitcoin', 'litecoin', 'tron', 'xrp', 'ton', 'stellar', 'cardano', ...Object.keys(LEGACY_SEND)].includes(c.id);
        fillChain(`chain-${c.id}`, addr, `${bal} ${c.sym}`, [], explorer, canSend ? c.id : undefined);
      })
      .catch(() => fillChain(`chain-${c.id}`, '', `– ${c.sym}`, []));
  }
  evmAddress()
    .then((addr) => {
      evmAddr = addr;
      for (const c of EVM_CHAINS) {
        evmBalance(c.rpc, addr, c.sym)
          .catch(() => `– ${c.sym}`)
          .then(async (native) => {
            // Only assets actually held show up in the dropdown.
            const held: Held[] = [];
            if (!native.startsWith('–')) held.push({ symbol: c.sym, token: '', decimals: 18, balance: native.split(' ')[0] });
            const rows = await Promise.all(
              TOKEN_BALANCES.filter((t) => t.chain === c.id).map(async (t) => {
                const b = await tokenBalance(c.rpc, t.token, addr, t.decimals).catch(() => null);
                if (!b || Number(b) <= 0) return '';
                held.push({ symbol: t.symbol, token: t.token, decimals: t.decimals, balance: b });
                const icon = t.symbol.endsWith('USDC') ? `<span class="coin small" aria-hidden="true">${ICONS.USDC}</span>` : '';
                return `<div class="row between"><span class="chain-name">${icon}${esc(t.symbol)}</span><span>${esc(b)}</span></div>`;
              }),
            );
            holdings[c.id] = held;
            fillChain(`chain-${c.id}`, addr, native, rows.filter(Boolean), c.explorer ? c.explorer + addr : undefined, c.id);
          });
      }
    })
    .catch(() => {});
  await Promise.all([
    loadSessions().catch((e) => {
      const ul = document.getElementById('sessions');
      if (ul) ul.innerHTML = `<li class="error small">Couldn't show your agents: ${esc(String((e as Error)?.message ?? e))}</li>`;
    }),
    loadPending(),
    loadQueued(),
  ]);
}

type QueuedItem = { wallet: string; key_id: string; action: string; created_ms: number; ready_at_ms: number };

/** Plain words for a queued action, decoded on this page. */
function explainAction(json: string): string {
  const a = JSON.parse(json);
  if (a.type === 'evm_tx') return explainEvm(JSON.parse(core.describe_evm_tx(JSON.stringify(a.tx))));
  if (a.type === 'near_call') return `Call ${esc(a.method)} on ${esc(a.receiver_id)} with ${formatNear(a.deposit)} NEAR`;
  if (a.type === 'utxo_pay' && PAY_CHAINS[a.chain]) {
    const pays = a.outputs.map((o: any) => `${esc(fmtUnits(BigInt(o.value), payDecimals(a.chain), 8))} ${PAY_CHAINS[a.chain]} to ${copyable(utxoAddr(a.chain, o.script))}`).join(', ');
    return `Pay on ${esc(PAY_NAMES[a.chain])}: ${pays} (outputs include any change back to you)`;
  }
  return `<span class="error">Unrecognised action</span>`;
}

async function loadQueued() {
  const items = await view<[number, QueuedItem][]>(cfg.extension, 'list_queued', { wallet }).catch(() => [] as [number, QueuedItem][]);
  document.getElementById('queued-card')!.hidden = !items.length;
  const ul = document.getElementById('queued')!;
  const now = Date.now();
  ul.innerHTML = items
    .map(([id, q]) => {
      let words: string;
      try {
        words = explainAction(q.action);
      } catch (e) {
        words = `<span class="error">Can't decode: ${esc((e as Error).message ?? e)}</span>`;
      }
      const mins = Math.max(0, Math.ceil((q.ready_at_ms - now) / 60_000));
      const when = q.ready_at_ms > now ? `can run in ${mins} min (${esc(new Date(q.ready_at_ms).toLocaleTimeString())})` : 'ready: anyone can run it now';
      return `<li><div>${words}</div><div class="small muted">#${id} · agent ${esc(q.key_id)} · ${when}</div>
        <button class="danger small-btn" data-cancel="${id}">Cancel</button></li>`;
    })
    .join('');
  ul.querySelectorAll<HTMLButtonElement>('[data-cancel]').forEach((b) => {
    b.onclick = () => {
      const msg = core.call_message(cfg.chain_id, wallet, nonce(), createdAt(), cfg.extension, 'cancel_queued', JSON.stringify({ id: Number(b.dataset.cancel) }), '0', 20n);
      confirm(msg);
    };
  });
}

type Pending = { id: number; key_id: string; tx: Record<string, unknown>; reason: string; status: string };
type EvmDescribed = { chain_id: number; to: string; value: string; call: any; hash: string };

/** Plain words for an EVM transaction, decoded on this page. */
function explainEvm(d: EvmDescribed): string {
  const c = d.call;
  const on = `on ${esc(chainName(d.chain_id))}`;
  switch (c.kind) {
    case 'erc20_approve': return `Allow ${named(d.chain_id, c.spender)} to take up to ${amount(d.chain_id, c.token, c.amount)} ${on}`;
    case 'erc20_transfer': return `Send ${amount(d.chain_id, c.token, c.amount)} to <span class="mono">${esc(c.recipient)}</span> ${on}`;
    case 'aave_supply': return `Supply ${amount(d.chain_id, c.asset, c.amount)} to ${named(d.chain_id, c.pool)} for <span class="mono">${esc(c.on_behalf_of)}</span> ${on}`;
    case 'aave_withdraw': return `Withdraw ${amount(d.chain_id, c.asset, c.amount)} from ${named(d.chain_id, c.pool)} to <span class="mono">${esc(c.to)}</span> ${on}`;
    case 'aave_repay': return `Repay ${amount(d.chain_id, c.asset, c.amount)} to ${named(d.chain_id, c.pool)} for <span class="mono">${esc(c.on_behalf_of)}</span> ${on}`;
    case 'uniswap_swap': return `Swap ${amount(d.chain_id, c.token_in, c.amount_in)} for at least ${amount(d.chain_id, c.token_out, c.min_out)} on ${named(d.chain_id, c.router)}, to <span class="mono">${esc(c.recipient)}</span> ${on}`;
    case 'native_transfer': return `Send ${esc(d.value)} wei to <span class="mono">${esc(d.to)}</span> ${on}`;
    default: return `<span class="error">Unrecognised call ${esc(c.selector)} to ${esc(c.contract)} ${on}</span>`;
  }
}

async function loadPending() {
  const items = (await relayer<Pending[]>(`/pending?wallet=${encodeURIComponent(wallet)}`).catch(() => [] as Pending[])).filter((p) => p.status === 'pending');
  const card = document.getElementById('pending-card')!;
  card.hidden = !items.length;
  const alert = document.getElementById('pending-alert');
  if (alert) {
    alert.hidden = !items.length;
    document.getElementById('pending-alert-text')!.textContent = `${items.length} request${items.length === 1 ? '' : 's'} from your agents need${items.length === 1 ? 's' : ''} your approval.`;
  }
  const ul = document.getElementById('pending')!;
  ul.innerHTML = items
    .map((p) => {
      let words: string;
      try {
        words = explainEvm(JSON.parse(core.describe_evm_tx(JSON.stringify(p.tx))));
      } catch (e) {
        words = `<span class="error">Can't decode: ${esc((e as Error).message ?? e)}</span>`;
      }
      return `<li><div>${words}</div><div class="small muted">Agent ${esc(p.key_id)} · refused: ${esc(p.reason)}</div>
        <div class="row"><button class="primary small-btn" data-approve="${p.id}">Review</button><button class="secondary small-btn" data-reject="${p.id}">Reject</button></div></li>`;
    })
    .join('');
  ul.querySelectorAll<HTMLButtonElement>('[data-approve]').forEach((b) => {
    b.onclick = () => {
      const p = items.find((i) => i.id === Number(b.dataset.approve))!;
      const tx = JSON.stringify(p.tx);
      const msg = core.owner_evm_sign_message(cfg.chain_id, wallet, nonce(), createdAt(), cfg.signer, tx);
      confirm(msg, { pendingId: p.id, tx });
    };
  });
  ul.querySelectorAll<HTMLButtonElement>('[data-reject]').forEach((b) => {
    b.onclick = async () => {
      await relayer(`/pending/${b.dataset.reject}/reject`, {});
      await loadPending();
    };
  });
}

/** A cap without "a day", for lists already headed "Daily limits" ("50 USDC on Base"). */
function capShort(c: { asset: string; daily: string }): string {
  const [kind, chain, token] = c.asset.split(':');
  if (kind === 'evm' && token) return `${amount(Number(chain), token, c.daily)} on ${esc(chainName(Number(chain)))}`;
  return capWords(c.asset, c.daily);
}
/** A daily cap in words ("50 USDC a day on Base"). */
function capText(c: { asset: string; daily: string }): string {
  const [kind, chain, token] = c.asset.split(':');
  if (kind === 'evm' && token) return `${amount(Number(chain), token, c.daily)} a day on ${esc(chainName(Number(chain)))}`;
  return `${capWords(c.asset, c.daily)} a day`;
}

async function loadSessions() {
  const ul = document.getElementById('sessions')!;
  // Read from the extension's index: what the chain says, not this browser.
  const rows = await view<[string, Session][]>(cfg.extension, 'list_sessions', { wallet }).catch(() => [] as [string, Session][]);
  renderProtocols(rows);
  document.querySelectorAll<HTMLButtonElement>('[data-protocol] [data-revoke]').forEach((b) => {
    b.onclick = () => confirm(core.call_message(cfg.chain_id, wallet, nonce(), createdAt(), cfg.extension, 'revoke_session', JSON.stringify({ key_id: b.dataset.revoke! }), '0', 20n));
  });
  if (!rows.length) {
    ul.innerHTML = '<li class="muted">No agents yet.</li>';
    return;
  }
  const now = Date.now();
  const active = rows.filter(([, s]) => !s.revoked && now < s.grant.expires_at_ms && now < s.last_used_ms + s.grant.inactivity_ms).length;
  const note = document.getElementById('agents-note');
  if (note) note.textContent = `${active} active`;
  ul.innerHTML = rows
    .map(([k, s]) => {
      const expired = now >= s.grant.expires_at_ms || now >= s.last_used_ms + s.grant.inactivity_ms;
      const state = s.revoked ? '<span class="pill bad">revoked</span>' : expired ? '<span class="pill bad">expired</span>' : '<span class="pill ok">active</span>';
      const venues = [
        ...s.grant.evm_venues.map((v) => `${esc(fnName(v.selector))} on ${named(v.chain_id, v.to)}, ${esc(chainName(v.chain_id))}`),
        ...s.grant.near_venues.map((v) => `${esc(v.receiver_id)}.${esc(v.method)}`),
        ...(s.grant.hl_markets ?? []).map((m) => `Hyperliquid ${esc(hlName(m.asset))} up to ${esc(m.max_leverage)}x`),
        ...((s.grant as any).utxo ?? []).map((u: any) => `payments on ${esc(PAY_NAMES[u.chain] ?? u.chain)} to ${u.recipients.map((r: string) => esc(short(utxoAddr(u.chain, r)))).join(', ') || 'no one'}`),
        ...grantChains(s.grant as any),
      ];
      const rules = grantRules(s.grant);
      const caps = s.grant.caps.map((c) => capShort(c).replace(/<\/?b>/g, ''));
      return `<li class="agent agent-card">
        <div class="app-row"><div class="app-logo">${AGENT_ICON}</div><div class="app-meta"><span class="name-line"><b>${esc(s.grant.label)}</b>${state}</span><span class="mono">Ends ${esc(dayText(s.grant.expires_at_ms))} · ${s.nonce} request${s.nonce === 1 ? '' : 's'}</span></div></div>
        <div class="rules">
          ${rules.slice(0, 4).map(([what, where]) => `<div><span class="yes">✓</span><span class="grow">${esc(what)}</span><span class="mono muted">${esc(where)}</span></div>`).join('') || '<div><span class="muted">Nothing</span></div>'}
          ${rules.length > 4 ? `<div><span class="muted">+${rules.length - 4} more</span></div>` : ''}
        </div>
        <div class="pills">
          ${caps.slice(0, 1).map((c: string) => `<span class="pill brass"><span class="dot"></span>${limitText(c)}</span>`).join('')}
          ${caps.length > 1 ? `<span class="pill soft">+${caps.length - 1} more limits</span>` : ''}
        </div>
        ${s.revoked || expired ? '' : `<div class="small muted" data-spent="${esc(k)}"></div>`}
        <div class="row between">
          <details class="more"><summary>Details</summary>
            <div class="small muted">Key ${esc(k)}</div>
            <div class="small">May: ${venues.length ? fold(venues) : 'nothing'}</div>
            <div class="small">Daily limits: ${caps.length ? fold(caps, 4) : 'no spending'}</div>
          </details>
          ${s.revoked ? '' : `<button class="danger small-btn" data-revoke="${esc(k)}">Revoke</button>`}
        </div>
      </li>`;
    })
    .join('');
  // What each live session has used of its caps today (the extension's own count).
  for (const [k, s] of rows) {
    const el = ul.querySelector<HTMLElement>(`[data-spent="${CSS.escape(k)}"]`);
    if (!el) continue;
    Promise.all(s.grant.caps.map((c) => view<string>(cfg.extension, 'spent_today', { wallet, key_id: k, asset: c.asset }).then((u) => [c, u] as const)))
      .then((used) => {
        const parts = used.filter(([, u]) => u !== '0').map(([c, u]) => `${capWords(c.asset, u)} of ${capWords(c.asset, c.daily)}`);
        el.textContent = '';
        el.innerHTML = parts.length ? `Used today: ${parts[0]}${parts.length > 1 ? ` and ${parts.length - 1} more` : ''}` : 'Nothing used today';
      })
      .catch(() => {});
  }
  ul.querySelectorAll<HTMLButtonElement>('[data-revoke]').forEach((b) => {
    b.onclick = () => {
      const key_id = b.dataset.revoke!;
      const msg = core.call_message(cfg.chain_id, wallet, nonce(), createdAt(), cfg.extension, 'revoke_session', JSON.stringify({ key_id }), '0', 20n);
      confirm(msg);
    };
  });
}

function grantForm() {
  const m = document.getElementById('modal')!;
  m.innerHTML = `
    <div class="backdrop"><form class="card sheet" id="grant">
      <h2>Add an agent or app</h2>
      <label>Name <input name="label" required value="Trading agent" maxlength="64"></label>
      <label>Key ID <input name="key_id" required value="agent-1" maxlength="64" pattern="[A-Za-z0-9_.\\-]+"></label>
      <label>Agent public key (ed25519, 64 hex characters) <input name="public_key" required pattern="[0-9a-fA-F]{64}" class="mono"></label>
      <p class="small muted">The key must live in Marvii's OutLayer module (TEE). Give the NEAR transaction of the OutLayer run that made it (<span class="mono">signer.keyProof</span> in the SDK); this page checks it on NEAR.</p>
      <div class="grid2">
        <label>Key's OutLayer run (tx hash) <input name="proof_tx" required class="mono" pattern="[1-9A-HJ-NP-Za-km-z]{43,44}"></label>
        <label>Run by (NEAR account) <input name="proof_sender" required class="mono"></label>
      </div>
      <div class="grid2">
        <label>Lasts (days) <input name="days" type="number" min="1" max="90" value="7"></label>
        <label>Stops if unused for (days) <input name="idle" type="number" min="1" max="90" value="3"></label>
      </div>
      <fieldset><legend>Venues on Base Sepolia</legend>
        ${VENUES.map((v) => `<label class="row check"><input type="checkbox" name="venue" value="${v.id}" style="width:auto"${v.id === 'aave' ? ' checked' : ''}> <span><b>${esc(v.name)}</b> <span class="muted small">${esc(v.what)}</span></span></label>`).join('')}
        <p class="small muted">Whatever these return (deposits, withdrawals, swap output) always goes to your own account; the extension refuses anything else.</p>
      </fieldset>
      <fieldset><legend>Another contract (optional)</legend>
        <div class="grid2">
          <label>Chain ID <input name="chain_id" type="number" value="84532"></label>
          <label>Function selector <input name="selector" placeholder="0x617ba037" pattern="0x[0-9a-fA-F]{8}" class="mono"></label>
        </div>
        <label>Contract <input name="to" placeholder="0x…" pattern="0x[0-9a-fA-F]{40}" class="mono"></label>
      </fieldset>
      <fieldset><legend>Daily cap</legend>
        <label>Token contract on that chain (empty for the native coin) <input name="token" placeholder="0x…" pattern="0x[0-9a-fA-F]{40}" class="mono"></label>
        <label>Most per day, in the token's smallest unit <input name="daily" required pattern="[0-9]+" value="50000000"></label>
      </fieldset>
      <fieldset><legend>Hyperliquid (testnet perps)</legend>
        <label class="row"><input type="checkbox" name="hl" style="width:auto"> Let it trade on Hyperliquid, signed by your account</label>
        <div class="grid2">
          <label>Market <select name="hl_asset">${Object.entries(HL_MARKETS).map(([a, n]) => `<option value="${a}"${n === 'ETH' ? ' selected' : ''}>${n} perps</option>`).join('')}</select></label>
          <label>Most leverage <input name="hl_lev" type="number" min="1" max="50" value="5"></label>
        </div>
        <label>Most order value per day (USD) <input name="hl_daily" inputmode="decimal" value="100"></label>
      </fieldset>
      <fieldset><legend>Payments on Bitcoin-style chains</legend>
        <div class="grid2">
          <label>Chain <select name="pay_chain"><option value="">None</option>${Object.entries(PAY_NAMES).map(([k, n]) => `<option value="${k}">${n}</option>`).join('')}</select></label>
          <label>Most per day, fees included <input name="pay_daily" inputmode="decimal" placeholder="0.001"></label>
        </div>
        <label>Who it may pay (addresses, comma-separated) <input name="pay_to" class="mono" placeholder="tb1q…"></label>
        <p class="small muted">Change always comes back to your account. Dogecoin isn't offered: its signatures don't cover amounts, so fees couldn't be capped.</p>
      </fieldset>
      <label class="row"><input type="checkbox" name="queue" style="width:auto"> Let it propose new recipients and contracts; they wait <span id="delay">the delay</span> and you can cancel</label>
      <p class="muted small">Otherwise the extension refuses anything else: other contracts, other functions, other recipients, or more than the cap.</p>
      <div class="row end"><button type="button" class="secondary" id="cancel">Cancel</button><button class="primary">Review</button></div>
    </form></div>`;
  document.getElementById('cancel')!.onclick = () => (m.innerHTML = '');
  view<number>(cfg.extension, 'queue_delay_ms', {})
    .then((ms) => (document.getElementById('delay')!.textContent = ms >= 3_600_000 ? `${ms / 3_600_000} hours` : `${ms / 60_000} minutes`))
    .catch(() => {});
  (document.getElementById('grant') as HTMLFormElement).onsubmit = async (ev) => {
    ev.preventDefault();
    const f = new FormData(ev.target as HTMLFormElement);
    const g = (k: string) => String(f.get(k) ?? '').trim();
    try {
      status('Checking where the agent key lives…');
      await checkTeeKey(g('public_key').toLowerCase(), { tx_hash: g('proof_tx'), sender: g('proof_sender') });
      status('');
    } catch (e) {
      status(String((e as Error).message ?? e), 'error');
      return;
    }
    const chain = Number(g('chain_id'));
    const now = Date.now();
    const grant = {
      public_key: g('public_key').toLowerCase(),
      label: g('label'),
      expires_at_ms: now + Number(g('days')) * 86_400_000,
      inactivity_ms: Number(g('idle')) * 86_400_000,
      near_venues: [],
      evm_venues: [
        ...VENUES.filter((v) => f.getAll('venue').includes(v.id)).flatMap((v) => v.selectors.map((selector) => ({ chain_id: v.chain, to: v.to, selector }))),
        ...(g('to') && g('selector') ? [{ chain_id: chain, to: g('to').toLowerCase(), selector: g('selector').toLowerCase() }] : []),
      ],
      near_recipients: [],
      evm_recipients: [],
      caps: [{ asset: g('token') ? `evm:${chain}:${g('token').toLowerCase()}` : `evm:${chain}`, daily: g('daily') }],
      queue_new_targets: f.get('queue') === 'on',
      hl_markets: [] as { asset: number; max_leverage: number }[],
      // The account's own EVM address: venue results must go here.
      evm_self: evmAddr ? evmAddr.toLowerCase() : undefined,
    };
    const payChain = g('pay_chain');
    if (payChain) {
      const own = utxoAccounts[payChain]?.address ?? legacyAccounts[payChain]?.address;
      if (!own) {
        alert('Your address on that chain is still loading. Try again in a moment.');
        return;
      }
      const recipients = g('pay_to').split(',').map((x) => x.trim()).filter(Boolean).map((a) => core.utxo_script(payChain, a));
      (grant as any).utxo = [{ chain: payChain, self_script: core.utxo_script(payChain, own), recipients }];
      grant.caps.push({ asset: `utxo:${payChain}`, daily: toUnits(g('pay_daily') || '0', 8) });
    }
    if (f.get('hl') === 'on') {
      grant.hl_markets.push({ asset: Number(g('hl_asset')), max_leverage: Number(g('hl_lev')) });
      grant.caps.push({ asset: 'hyperliquid:usd', daily: toUnits(g('hl_daily'), 6) });
    }
    const key_id = g('key_id');
    // 0.01 NEAR: the extension's storage deposit for a session.
    const msg = core.call_message(cfg.chain_id, wallet, nonce(), createdAt(), cfg.extension, 'grant_session', JSON.stringify({ key_id, grant }), '10000000000000000000000', 20n);
    confirm(msg);
  };
}

/** An output script as its address, or the script itself where this page
 *  can't encode the chain's addresses (Dogecoin, which the grant form
 *  doesn't offer). */
/** Bech32 (BIP-173) of raw bytes, for Cardano's addr_test… addresses. */
function bech32(hrp: string, data: Uint8Array): string {
  const CH = 'qpzry9x8gf2tvdw0s3jn54khce6mua7l';
  const words: number[] = [];
  let acc = 0, bits = 0;
  for (const b of data) {
    acc = (acc << 8) | b;
    bits += 8;
    while (bits >= 5) words.push((acc >> (bits -= 5)) & 31);
  }
  if (bits) words.push((acc << (5 - bits)) & 31);
  const polymod = (v: number[]) => {
    const G = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
    let chk = 1;
    for (const x of v) {
      const top = chk >> 25;
      chk = ((chk & 0x1ffffff) << 5) ^ x;
      for (let i = 0; i < 5; i++) if ((top >> i) & 1) chk ^= G[i];
    }
    return chk;
  };
  const hv = [...hrp].map((c) => c.charCodeAt(0) >> 5).concat([0], [...hrp].map((c) => c.charCodeAt(0) & 31));
  const pm = polymod([...hv, ...words, 0, 0, 0, 0, 0, 0]) ^ 1;
  const cs = [0, 1, 2, 3, 4, 5].map((i) => (pm >> (5 * (5 - i))) & 31);
  return `${hrp}1${[...words, ...cs].map((w) => CH[w]).join('')}`;
}

function utxoAddr(chain: string, script: string): string {
  if (chain === 'cardano' && /^[0-9a-f]+$/i.test(script)) return bech32('addr_test', Uint8Array.from(script.match(/../g)!.map((h) => parseInt(h, 16))));
  try {
    return core.utxo_address(chain, script);
  } catch {
    return `script ${script}`;
  }
}

/** The non-EVM parts of a grant, briefly, for the agents list. */
// ---- Apps & agents by protocol: each protocol's page lists the agents that
// can use it and what they may do there. Access comes from permission links;
// these pages are for seeing and revoking. ----

type Protocol = { id: string; name: string; chain: string; about: string; access: (g: any) => string[]; caps?: (asset: string, g: any) => boolean };
/** The chains a protocol's EVM venues are on, matched by name prefix. */
const evmChainsOf = (g: any, prefix: string) => new Set((g.evm_venues ?? []).filter((v: any) => (POOLS[`${v.chain_id}:${v.to}`] ?? '').startsWith(prefix)).map((v: any) => String(v.chain_id)));
/** Caps on the coin and tokens of the chains where the protocol's EVM venues are, and borrowing there. */
const evmCaps = (prefix: string) => (a: string, g: any) => {
  const [kind, chain] = a.split(':');
  const chains = evmChainsOf(g, prefix);
  return (kind === 'evm' && chains.has(chain)) || (a.startsWith('borrow:evm:') && chains.has(a.split(':')[2]));
};
const capsByPrefix = (...prefixes: string[]) => (a: string) => prefixes.some((p) => (p.endsWith(':') ? a.startsWith(p) : a === p));
const evmAt = (g: any, prefix: string) =>
  (g.evm_venues ?? []).filter((v: any) => (POOLS[`${v.chain_id}:${v.to}`] ?? '').startsWith(prefix)).map((v: any) => `${esc(fnName(v.selector))} on ${named(v.chain_id, v.to)}, ${esc(chainName(v.chain_id))}`);
const solAt = (g: any, ...programs: string[]) =>
  (g.sol?.venues ?? []).filter((v: any) => programs.includes(v.program)).map((v: any) => `${esc(SOL_PROGRAMS[v.program] ?? short(v.program))}${v.scope === v.program ? '' : ` (${esc(short(v.scope))})`}`);
const nearAt = (g: any, test: (v: any) => boolean) => (g.near_venues ?? []).filter(test).map((v: any) => `${esc(v.method)} on ${esc(v.receiver_id)}`);
const tronAt = (g: any, prefix: string) =>
  (g.tron?.venues ?? []).filter((v: any) => (TRON_NAMES[v.contract] ?? '').startsWith(prefix)).map((v: any) => `${esc(fnName(v.selector))} on ${esc(TRON_NAMES[v.contract])}`);
const PROTOCOLS: Protocol[] = [
  { id: 'aave', name: 'Aave V3', chain: 'EVM', about: 'Lending. Supply, withdraw, repay and borrow; what comes back always goes to your account.', access: (g) => evmAt(g, 'Aave'), caps: evmCaps('Aave') },
  { id: 'uniswap', name: 'Uniswap V3', chain: 'EVM', about: 'Token swaps; the output always goes to your account.', access: (g) => evmAt(g, 'Uniswap'), caps: evmCaps('Uniswap') },
  { id: 'lido', name: 'Lido', chain: 'EVM', about: 'Stake ETH for stETH, wrap and unwrap wstETH, and withdraw back to ETH through the withdrawal queue.', access: (g) => evmAt(g, 'Lido'), caps: evmCaps('Lido') },
  { id: 'seaport', name: 'Seaport (OpenSea)', chain: 'EVM', about: 'List your NFTs for sale, never below the price floor you set, paid to you.', access: (g) => [...evmAt(g, 'Seaport'), ...(g.evm_nft_floor && g.evm_nft_floor !== '0' ? [`listings from ${esc(fmtUnits(BigInt(g.evm_nft_floor), 18))} in the chain's coin`] : [])], caps: (a, g) => a.startsWith('nft:evm:') || evmCaps('Seaport')(a, g) },
  { id: 'polymarket', name: 'Polymarket', chain: 'EVM', about: 'Prediction-market orders on Polygon, signed by your account: buys paid in USDC.e, sells within a daily limit.', access: (g) => evmAt(g, 'Polymarket'), caps: (a, g) => a === 'polymarket:sell' || evmCaps('Polymarket')(a, g) },
  { id: 'hyperliquid', name: 'Hyperliquid', chain: 'Hyperliquid', about: 'Perps orders, cancels and leverage, on the markets and up to the leverage you allow.', access: (g) => (g.hl_markets ?? []).map((m: any) => `${esc(hlName(m.asset))} perps up to ${esc(m.max_leverage)}x`), caps: capsByPrefix('hyperliquid:') },
  { id: 'ref', name: 'Ref Finance', chain: 'NEAR', about: 'Token swaps on NEAR, only through the pools you allow.', access: (g) => nearAt(g, (v) => /ref-finance/.test(v.receiver_id) || String(v.method).startsWith('pool:')), caps: capsByPrefix('near', 'nep141:') },
  { id: 'rhea', name: 'Rhea Lending', chain: 'NEAR', about: 'Lending on NEAR (formerly Burrow). Borrowing counts against its own limit.', access: (g) => nearAt(g, (v) => /burrow/.test(v.receiver_id)), caps: capsByPrefix('near', 'nep141:', 'borrow:near:') },
  { id: 'nearx', name: 'Stader NearX', chain: 'NEAR', about: 'Liquid staking: stake NEAR for NearX, unstake and withdraw.', access: (g) => nearAt(g, (v) => /nearx/.test(v.receiver_id)), caps: capsByPrefix('near', 'nep141:') },
  { id: 'jupiter', name: 'Jupiter', chain: 'Solana', about: 'Token swaps across Solana; output to your account, no extra fee, slippage at most 3%.', access: (g) => solAt(g, 'JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4'), caps: capsByPrefix('sol', 'spl:') },
  { id: 'phoenix-perps', name: 'Phoenix perps', chain: 'Solana', about: 'Perpetual futures. Each agent trades its own subaccount with a key that can trade but never withdraw.', access: (g) => (g.sol?.perps?.length ? [perpsWords(g.sol.perps)] : []), caps: capsByPrefix('phoenix:', 'sol', 'spl:EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v') },
  { id: 'phoenix', name: 'Phoenix spot', chain: 'Solana', about: 'Order-book swaps; both sides settle in your own token accounts.', access: (g) => solAt(g, 'PhoeNiXZ8ByJGLkxNfZRnkUfjvmuYqLR89jjFHGqdXY'), caps: capsByPrefix('sol', 'spl:') },
  { id: 'jito', name: 'Jito', chain: 'Solana', about: 'Liquid staking: SOL for JitoSOL.', access: (g) => solAt(g, 'SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy', 'DPoo15wWDqpPJJtS2MUZ49aRxqz5ZaaJCJP4z8bLuib'), caps: capsByPrefix('sol', 'spl:') },
  { id: 'kamino', name: 'Kamino', chain: 'Solana', about: 'Lending on Solana. Borrowing counts against its own limit.', access: (g) => solAt(g, 'KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD'), caps: capsByPrefix('sol', 'spl:', 'borrow:solana:') },
  { id: 'tensor', name: 'Tensor', chain: 'Solana', about: 'NFT listings and buys, never below the price floor you set.', access: (g) => [...solAt(g, 'TCMPhJdwDryooaGtiocG1u3xcYbRpiJzb283XfCZsDp'), ...(g.sol?.nft_floor && g.sol.nft_floor !== '0' ? [`listings from ${esc(fmtUnits(BigInt(g.sol.nft_floor), 9))} SOL`] : [])], caps: capsByPrefix('sol', 'nft:solana') },
  { id: 'justlend', name: 'JustLend', chain: 'TRON', about: 'Lending on TRON, and liquid staking for sTRX.', access: (g) => [...tronAt(g, 'JustLend'), ...tronAt(g, 'sTRX')], caps: capsByPrefix('trx', 'trc20:', 'borrow:tron:') },
  { id: 'sunswap', name: 'SunSwap', chain: 'TRON', about: 'Token swaps on TRON; output to your account.', access: (g) => tronAt(g, 'SunSwap'), caps: capsByPrefix('trx', 'trc20:') },
  { id: 'tonstakers', name: 'Tonstakers', chain: 'TON', about: 'Liquid staking: TON for tsTON, and unstaking back to your account.', access: (g) => (g.ton?.lst ?? []).map((l: any) => `pool ${esc(short(l.pool))}`), caps: capsByPrefix('ton', 'jetton:') },
  { id: 'evaa', name: 'EVAA', chain: 'TON', about: 'Lending on TON. Every withdrawal counts against the borrowing limit.', access: (g) => (g.ton?.evaa ?? []).map((m: string) => `master ${esc(short(m))}`), caps: capsByPrefix('ton', 'jetton:', 'borrow:ton:') },
  { id: 'xrpl-dex', name: 'XRP Ledger DEX and AMM', chain: 'XRP Ledger', about: "The ledger's own order book and AMM pools.", access: (g) => [g.xrpl?.dex && 'order book offers', g.xrpl?.amm && 'AMM deposits and withdrawals'].filter(Boolean) as string[], caps: capsByPrefix('xrp', 'xrpl:') },
  { id: 'stellar-dex', name: 'Stellar DEX and AMM', chain: 'Stellar', about: "Stellar's own order book, path payments and liquidity pools.", access: (g) => [g.stellar?.dex && 'order book offers and path payments', g.stellar?.amm && 'liquidity pools'].filter(Boolean) as string[], caps: capsByPrefix('xlm', 'stellar:') },
  { id: 'blend', name: 'Blend', chain: 'Stellar', about: 'Lending on Stellar.', access: (g) => (g.stellar?.blend ?? []).map((p: string) => `pool ${esc(short(p))}`), caps: capsByPrefix('xlm', 'stellar:', 'soroban:', 'borrow:stellar:') },
  { id: 'nfts', name: 'NFT transfers', chain: 'Other chains', about: 'Sending your NFTs to addresses you list (TON, TRON, XRP Ledger).', access: (g) => [g.ton?.nft && 'TON NFTs', g.tron?.nfts?.length && `TRON NFTs (${g.tron.nfts.length} collections)`, g.xrpl?.nft && 'XRP Ledger NFTs'].filter(Boolean) as string[], caps: capsByPrefix('nft:') },
  { id: 'payments', name: 'Payments', chain: 'Other chains', about: 'Paying the recipients you list, on Bitcoin, Litecoin, Dogecoin, Bitcoin Cash, Zcash and Cardano. Change always comes back to you.', access: (g) => (g.utxo ?? []).map((u: any) => `${esc(PAY_NAMES[u.chain] ?? u.chain)} to ${u.recipients.map((r: string) => esc(short(utxoAddr(u.chain, r)))).join(', ') || 'no one'}`), caps: capsByPrefix('utxo:') },
];

/** The protocols a grant opens, as chips, plus contracts no named protocol covers. */
function protocolChips(g: any, max = Infinity): string {
  const chips = PROTOCOLS.filter((p) => p.access(g).length).map((p) => `<span class="chip">${esc(p.name)}<small>${esc(p.chain)}</small></span>`);
  const otherEvm = (g.evm_venues ?? []).filter((v: any) => !POOLS[`${v.chain_id}:${String(v.to).toLowerCase()}`]).length;
  const otherNear = (g.near_venues ?? []).filter((v: any) => !/ref-finance|burrow|nearx/.test(v.receiver_id) && !String(v.method).startsWith('pool:')).length;
  const otherTron = (g.tron?.venues ?? []).filter((v: any) => !TRON_NAMES[v.contract]).length;
  if (otherEvm) chips.push(`<span class="chip">${otherEvm} other contract call${otherEvm === 1 ? '' : 's'}<small>EVM</small></span>`);
  if (otherNear) chips.push(`<span class="chip">${otherNear} other contract call${otherNear === 1 ? '' : 's'}<small>NEAR</small></span>`);
  if (otherTron) chips.push(`<span class="chip">${otherTron} other contract call${otherTron === 1 ? '' : 's'}<small>TRON</small></span>`);
  if (!chips.length) return '<span class="muted">Nothing</span>';
  const shown = chips.slice(0, max);
  return `<div class="chips">${shown.join('')}${chips.length > shown.length ? `<span class="chip more">+${chips.length - shown.length} more</span>` : ''}</div>`;
}
/** Everyone a grant lets the agent pay directly, on any chain. */
function payees(g: any): string[] {
  return [
    ...(g.evm_recipients ?? []), ...(g.near_recipients ?? []), ...(g.sol?.recipients ?? []), ...(g.ton?.recipients ?? []),
    ...(g.tron?.recipients ?? []), ...(g.xrpl?.recipients ?? []), ...(g.stellar?.recipients ?? []),
    ...(g.utxo ?? []).flatMap((u: any) => u.recipients.map((r: string) => utxoAddr(u.chain, r))),
  ].map(String);
}
/** "20 Nov 2026" */
const dayText = (ms: number) => new Date(ms).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' });
/** What each protocol lets an agent do, as a short phrase for the request card. */
const PROTOCOL_VERBS: Record<string, string> = {
  aave: 'Lend and borrow on Aave', uniswap: 'Swap on Uniswap', lido: 'Stake ETH with Lido', seaport: 'List NFTs on Seaport', polymarket: 'Trade on Polymarket',
  ref: 'Swap on Ref', rhea: 'Lend and borrow on Rhea', nearx: 'Stake NEAR with NearX', jupiter: 'Swap on Jupiter', 'phoenix-perps': 'Trade perps on Phoenix',
  phoenix: 'Swap on Phoenix', jito: 'Stake SOL with Jito', kamino: 'Lend and borrow on Kamino', tensor: 'Trade NFTs on Tensor', justlend: 'Lend and stake on JustLend',
  sunswap: 'Swap on SunSwap', tonstakers: 'Stake TON with Tonstakers', evaa: 'Lend on EVAA', 'xrpl-dex': 'Trade on the XRP Ledger', 'stellar-dex': 'Trade on Stellar',
  blend: 'Lend and borrow on Blend', nfts: 'Send NFTs to saved addresses',
};
/** The rows of a grant's request card: [what, where]; `only` limits them to one protocol. */
function grantRules(g: any, only?: string): [string, string][] {
  const rows: [string, string][] = [];
  for (const p of PROTOCOLS) {
    if (only && p.id !== only) continue;
    if (p.id === 'payments' && only) {
      for (const u of g.utxo ?? []) rows.push([`Pay ${u.recipients.length} saved address${u.recipients.length === 1 ? '' : 'es'}`, PAY_NAMES[u.chain] ?? u.chain]);
      continue;
    }
    if (p.id === 'payments' || !p.access(g).length) continue;
    if (p.id === 'hyperliquid') {
      for (const m of g.hl_markets ?? []) rows.push([`Trade ${hlName(m.asset)} perps on Hyperliquid`, `≤ ${m.max_leverage}x`]);
      continue;
    }
    if (p.id === 'phoenix-perps') {
      for (const d of g.sol.perps) for (const m of d.markets) rows.push([`Trade ${PERP_MARKETS[m.market]?.[0] ?? 'perps'}-PERP on Phoenix`, `≤ ${perpSize(m.market, m.max_base_lots).replace(/<[^>]+>/g, '')} an order`]);
      continue;
    }
    rows.push([PROTOCOL_VERBS[p.id] ?? p.name, p.chain === 'Other chains' ? '' : p.chain]);
  }
  const otherEvm = (g.evm_venues ?? []).filter((v: any) => !POOLS[`${v.chain_id}:${String(v.to).toLowerCase()}`]).length;
  const otherNear = (g.near_venues ?? []).filter((v: any) => !/ref-finance|burrow|nearx/.test(v.receiver_id) && !String(v.method).startsWith('pool:')).length;
  const otherTron = (g.tron?.venues ?? []).filter((v: any) => !TRON_NAMES[v.contract]).length;
  if (only) return rows;
  if (otherEvm + otherNear + otherTron) rows.push([`Call ${otherEvm + otherNear + otherTron} other contract function${otherEvm + otherNear + otherTron === 1 ? '' : 's'}`, 'see details']);
  const pay = payees(g).length;
  if (pay) rows.push([`Pay ${pay} saved address${pay === 1 ? '' : 'es'}`, '']);
  return rows;
}
/** A daily limit as a pill's words: "Up to 50 USDC on Base a day", "Borrow up to 2 USDC on Base a day". */
const limitText = (c: string) => (c.startsWith('borrowing ') ? `Borrow up to ${c.slice(10)} a day` : `Up to ${c} a day`);
const AGENT_ICON = `<svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="4" y="8" width="16" height="12" rx="3.5"/><path d="M12 4v4"/><circle cx="12" cy="3.5" r="1"/><path d="M9 13v1.5M15 13v1.5M9.5 17.5h5"/></svg>`;
/** A grant as a request card, like the landing page's phone: who asks, what it may do, its limits. */
function grantRequest(g: any, name: string, fromLink: boolean): string {
  const rows = grantRules(g);
  const caps = (g.caps ?? []).map((c: any) => capShort(c).replace(/<\/?b>/g, ''));
  const acct = document.getElementById('acct-name')?.textContent?.trim() || short(wallet);
  const days = Math.max(1, Math.round((g.expires_at_ms - Date.now()) / 86_400_000));
  return `<div class="req">
    <div class="app-row"><div class="app-logo">${AGENT_ICON}</div><div class="app-meta"><span><b>${esc(name)}</b> wants a session key</span><span class="mono">for ${esc(acct)}</span></div></div>
    <div class="req-title">Let this agent act for you?</div>
    <div class="rules">
      ${rows.map(([what, where]) => `<div><span class="yes">✓</span><span class="grow">${esc(what)}</span><span class="mono muted">${esc(where)}</span></div>`).join('')}
      <div><span class="no">✕</span><span class="muted">Send your money anywhere you didn't list</span></div>
    </div>
    <div class="pills">
      ${caps.slice(0, 2).map((c: string) => `<span class="pill brass"><span class="dot"></span>${limitText(c)}</span>`).join('')}
      ${caps.length > 2 ? `<span class="pill soft">+${caps.length - 2} more limits</span>` : ''}
      <span class="pill soft">${days} day${days === 1 ? '' : 's'}</span>
      <span class="pill soft">Revoke any time</span>
    </div>
    ${g.queue_new_targets ? '<p class="small warn">It may also propose new recipients and contracts. Each waits the delay, and you can cancel it.</p>' : ''}
    ${fromLink ? `<p class="small muted">"${esc(name)}" is the app's own name for itself: approve only if you started this. <span class="tee">✓ Its key was made inside Marvii's TEE, checked on NEAR.</span></p>` : ''}
  </div>`;
}

/** A page per protocol, filled in by renderProtocols. */
function protocolViews(): string {
  return PROTOCOLS.map((p) => `
    <section class="view" data-view="p-${p.id}">
      <a class="back" href="#/agents">‹ Apps &amp; agents</a>
      <section class="card">
        <div class="row between"><h1>${esc(p.name)}</h1><span class="pill">${esc(p.chain)}</span></div>
        <p class="muted small">${esc(p.about)}</p>
        <div class="sec-label">Agents that can use it</div>
        <ul class="list" data-protocol="${p.id}"><li class="muted">Loading…</li></ul>
        <p class="muted small">Revoking stops an agent everywhere, not only here.</p>
      </section>
    </section>`).join('');
}

/** Fills the protocol dropdown and each protocol's page from the sessions. */
function renderProtocols(rows: [string, Session][]) {
  const now = Date.now();
  const live = rows.filter(([, s]) => !s.revoked && now < s.grant.expires_at_ms && now < s.last_used_ms + s.grant.inactivity_ms);
  const users = (p: Protocol) => live.filter(([, s]) => p.access(s.grant).length);
  for (const p of PROTOCOLS) {
    const ul = document.querySelector<HTMLElement>(`[data-protocol="${p.id}"]`);
    if (!ul) continue;
    const us = users(p);
    ul.innerHTML = us.length
      ? us.map(([k, s]) => {
          const rules = grantRules(s.grant, p.id);
          const caps = s.grant.caps.filter((c) => p.caps?.(c.asset, s.grant) ?? true).map((c) => capShort(c).replace(/<\/?b>/g, ''));
          return `<li class="agent agent-card">
          <div class="app-row"><div class="app-logo">${AGENT_ICON}</div><div class="app-meta"><span class="name-line"><b>${esc(s.grant.label)}</b><span class="pill ok">active</span></span><span class="mono">Ends ${esc(dayText(s.grant.expires_at_ms))}</span></div></div>
          <div class="rules">${rules.map(([what, where]) => `<div><span class="yes">✓</span><span class="grow">${esc(what)}</span><span class="mono muted">${esc(where)}</span></div>`).join('')}</div>
          <div class="pills">${caps.length ? caps.map((c) => `<span class="pill brass"><span class="dot"></span>${limitText(c)}</span>`).join('') : '<span class="pill soft">No limits that apply here</span>'}</div>
          <div class="row between">
            <details class="more"><summary>Details</summary><div class="small">${p.access(s.grant).join('; ')}</div></details>
            <button class="danger small-btn" data-revoke="${esc(k)}">Revoke</button>
          </div>
        </li>`;
        }).join('')
      : `<li class="muted">No agent can use ${esc(p.name)}.</li>`;
  }
}

function grantChains(g: any): string[] {
  const out: string[] = [];
  if (g.sol) out.push(`Solana: ${(g.sol.venues ?? []).map((v: any) => esc(SOL_PROGRAMS[v.program] ?? short(v.program))).join(', ') || 'payments'}${g.sol.nft_floor && g.sol.nft_floor !== '0' ? `, Tensor listings from ${esc(fmtUnits(BigInt(g.sol.nft_floor), 9))} SOL` : ''}${g.sol.perps?.length ? ', Phoenix perps' : ''}`);
  if (g.ton) out.push(`TON: ${[g.ton.recipients?.length && 'payments', g.ton.jettons?.length && 'jettons', g.ton.evaa?.length && 'EVAA lending', g.ton.lst?.length && 'Tonstakers staking', g.ton.nft && 'NFTs'].filter(Boolean).join(', ') || 'nothing'}`);
  if (g.tron) out.push(`TRON: ${(g.tron.venues ?? []).length} contract calls${g.tron.nfts?.length ? ', NFTs' : ''}`);
  if (g.xrpl) out.push(`XRP Ledger: ${[g.xrpl.dex && 'order book', g.xrpl.amm && 'AMM', g.xrpl.nft && 'NFTs'].filter(Boolean).join(', ') || 'payments'}`);
  if (g.stellar) out.push(`Stellar: ${[g.stellar.dex && 'order book', g.stellar.amm && 'AMM', g.stellar.blend?.length && 'Blend lending'].filter(Boolean).join(', ') || 'payments'}`);
  if (g.evm_nft_floor && g.evm_nft_floor !== '0') out.push(`Seaport listings from ${esc(fmtUnits(BigInt(g.evm_nft_floor), 18))}`);
  return out;
}

/** Solana programs the extension knows (sol.rs), by name. */
const SOL_PROGRAMS: Record<string, string> = {
  SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy: 'Jito staking',
  DPoo15wWDqpPJJtS2MUZ49aRxqz5ZaaJCJP4z8bLuib: 'Jito staking (devnet)',
  KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD: 'Kamino lending',
  TCMPhJdwDryooaGtiocG1u3xcYbRpiJzb283XfCZsDp: 'Tensor NFT marketplace',
  JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4: 'Jupiter swaps (mainnet)',
  PhoeNiXZ8ByJGLkxNfZRnkUfjvmuYqLR89jjFHGqdXY: 'Phoenix order book (mainnet)',
  EtrnLzgbS7nMMy5fbD42kXiUzGg8XQzJ972Xtk1cjWih: 'Phoenix perps (mainnet)',
};

/** Phoenix perps markets (perp asset map on mainnet): name and base-lot decimals. */
const PERP_MARKETS: Record<string, [string, number]> = {
  '71Si24E4uc3oCaPbPZTozC1ptSNNqygjjebxSmErSsC2': ['SOL', 2],
  AXFz1MuzMUBHi5UKJuK3FDCQ73o3rSzubGU2mPr4LLU7: ['BTC', 4],
  '9u7aqptdRFbsnnoHtjK13E5JkeM14EW5fAKTRPidVF88': ['ETH', 3],
};
/** A size on a Phoenix perps market in base units (lots / 10^decimals). */
function perpSize(market: string, lots: string): string {
  const m = PERP_MARKETS[market];
  return m ? `${esc(fmtUnits(BigInt(lots), m[1], m[1]))} ${m[0]}` : `${esc(lots)} lots on Phoenix market ${esc(short(market))}`;
}
/** Plain words for the grant's Phoenix perps desks. */
function perpsWords(desks: any[]): string {
  return desks
    .map((d: any) => `trade <b>Phoenix perps</b> on its own isolated subaccount ${esc(String(d.sub))}, through a trading key of your account's (${copyable(d.trader_key)}) that Phoenix lets trade it and never withdraw: ${d.markets.map((m: any) => `${PERP_MARKETS[m.market]?.[0] ?? short(m.market)}-PERP up to ${perpSize(m.market, m.max_base_lots)} an order`).join(', ')}, limit orders that fill at once or not at all. It may move your USDC into Phoenix and back, only between your own accounts`)
    .join('; ');
}

// ---- Permission links (near/sdk permissionLink): #/connect?r=<request> ----

function base58Bytes(s: string): Uint8Array {
  const A = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
  let n = 0n;
  for (const c of s) {
    const i = A.indexOf(c);
    if (i < 0) throw new Error('not base58');
    n = n * 58n + BigInt(i);
  }
  const hex = n.toString(16).padStart(2, '0');
  const body = (hex.length % 2 ? '0' : '') + hex;
  const lead = (s.match(/^1*/) ?? [''])[0].length;
  return Uint8Array.from(('00'.repeat(lead) + body).match(/../g)!.map((h) => parseInt(h, 16)));
}

/**
 * An app's request for a session key, from a permission link. The page
 * fills in the account's own addresses, then shows the grant on the usual
 * Confirm screen, which explains every part of it (or refuses to approve a
 * part it can't explain). The app's name is its own claim, shown as such.
 */
/** Marvii's published OutLayer modules (sha256 of the WASM build) that may
 *  hold agent keys, and OutLayer's contract, per network. */
const TEE_MODULES = ['551085926472e0a1236afa1c7461e63c87413496c63cf77645890e886433775b', '02954a8c512a5445eba70802579d7e6d09752c9d0ecc4d66b5cb237685927860', 'd6de908f4e974eaa1205a2523d05ccd31ac6a448694dac5313ebd30eab1d8543'];
const OUTLAYER: Record<string, string> = { testnet: 'outlayer.testnet' };

/**
 * Agents may hold their keys only inside Marvii's OutLayer module. The link
 * names the NEAR transaction of the OutLayer run that made the key; on NEAR
 * that run must be a call to OutLayer's contract running one of our modules
 * on `{"operation":"public_key"}`, and its result must be this key. (OutLayer
 * derives a different key per module and operator, so the run, not a list of
 * keys, is the proof.)
 */
async function checkTeeKey(publicKey: string, proof: { tx_hash?: string; sender?: string } | undefined) {
  const no = (why: string) => new Error(`this agent's key isn't held in Marvii's TEE module (${why}). Agents must keep their keys in OutLayer with OutLayerSigner`);
  const outlayer = OUTLAYER[cfg.chain_id];
  if (!outlayer) throw new Error('no TEE module is published for this network yet');
  if (!proof?.tx_hash || !proof.sender) throw no('the link has no key proof');
  if (!/^[1-9A-HJ-NP-Za-km-z]{43,44}$/.test(proof.tx_hash) || !/^[a-z0-9_.-]{2,64}$/.test(proof.sender)) throw no('bad key proof');
  const r = await fetch(cfg.rpc, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tx', params: { tx_hash: proof.tx_hash, sender_account_id: proof.sender, wait_until: 'FINAL' } }),
    signal: AbortSignal.timeout(20000),
  }).then((r) => r.json());
  const t = r.result?.transaction;
  if (!t) throw no('its OutLayer run was not found on NEAR');
  const fc = t.actions?.[0]?.FunctionCall;
  if (t.receiver_id !== outlayer || t.actions.length !== 1 || fc?.method_name !== 'request_execution') throw no('that transaction is not an OutLayer run');
  const args = JSON.parse(new TextDecoder().decode(Uint8Array.from(atob(fc.args), (c) => c.charCodeAt(0))));
  if (!TEE_MODULES.includes(args.source?.WasmUrl?.hash) || args.source?.WasmUrl?.build_target !== 'wasm32-wasip2') throw no('it ran a different module');
  if (args.input_data !== '{"operation":"public_key"}') throw no('that run did not make a key');
  const v = r.result.status?.SuccessValue;
  const out = v ? JSON.parse(new TextDecoder().decode(Uint8Array.from(atob(v), (c) => c.charCodeAt(0)))) : null;
  if (out?.public_key !== publicKey) throw no('that run made a different key');
}

async function connectFromLink() {
  const m = location.hash.match(/^#\/connect\?r=([A-Za-z0-9_-]+)$/);
  if (!m) return;
  const b64 = m[1].replace(/-/g, '+').replace(/_/g, '/');
  const req = JSON.parse(new TextDecoder().decode(Uint8Array.from(atob(b64 + '='.repeat((4 - (b64.length % 4)) % 4)), (c) => c.charCodeAt(0))));
  window.history.replaceState(null, '', location.pathname + location.search + '#/agents');
  route();
  if (req.v !== 1) throw new Error('unknown request version');
  if (!/^[0-9a-f]{64}$/.test(req.public_key ?? '')) throw new Error('bad agent key');
  if (!/^[a-z0-9_.-]{1,64}$/.test(req.key_id ?? '')) throw new Error('bad key name');
  const name = String(req.app?.name ?? '').slice(0, 64);
  if (!name) throw new Error('the app has no name');
  status('Checking where the agent key lives…');
  await checkTeeKey(req.public_key, req.key_proof);
  const days = Number(req.days ?? 7), idle = Number(req.idle_days ?? 3);
  if (!(days >= 1 && days <= 90 && idle >= 1 && idle <= 90)) throw new Error('lasting 1 to 90 days');
  if (typeof req.grant !== 'object' || req.grant === null) throw new Error('no permissions asked for');
  for (const k of ['public_key', 'label', 'expires_at_ms', 'inactivity_ms']) if (k in req.grant) throw new Error(`the link may not set ${k}`);
  let returnUrl: URL | null = null;
  if (req.return_url) {
    returnUrl = new URL(req.return_url);
    if (returnUrl.protocol !== 'https:' && returnUrl.hostname !== 'localhost') throw new Error('the return address must be https');
  }
  // The account's own addresses, as the request's placeholders name them.
  let text = JSON.stringify(req.grant);
  status('Loading your addresses…');
  const fill: Record<string, () => string> = {
    '{evm_self}': () => evmAddr.toLowerCase(),
    '{sol_self}': () => solana.address,
    '{ton_self}': () => ton.address,
    '{tron_self}': () => tron.address,
    '{stellar_self}': () => stellar.address,
    '{xrp_self}': () => xrp.address,
    '{xrp_pk}': () => {
      if (!xrp.key) return '';
      const xy = base58Bytes(xrp.key.replace(/^secp256k1:/, ''));
      return [xy[63] & 1 ? 3 : 2, ...xy.slice(0, 32)].map((b) => b.toString(16).padStart(2, '0')).join('');
    },
  };
  for (const [ph, val] of Object.entries(fill)) {
    if (!text.includes(ph)) continue;
    const v = await until(async () => val() || null, 30000);
    if (!v) throw new Error(`your ${ph.slice(1, -1).replace('_self', '').replace('_pk', '')} address didn't load; try again`);
    text = text.split(ph).join(v);
  }
  // Phoenix perps: the account's trading key for isolated subaccount n
  // (Chain Signatures, path phoenix-n), which the extension signs orders with.
  for (const m of new Set(text.match(/\{phoenix_trader_\d{1,2}\}/g) ?? [])) {
    const k = await view<string>(cfg.signer, 'derived_public_key', { path: `phoenix-${m.slice(16, -1)}`, predecessor: wallet, domain_id: 1 });
    text = text.split(m).join(core.chain_address('solana', k));
  }
  if (/\{[a-z_]+\}/.test(text)) throw new Error(`unknown placeholder ${text.match(/\{[a-z_]+\}/)![0]}`);
  const now = Date.now();
  const grant = {
    near_venues: [], evm_venues: [], near_recipients: [], evm_recipients: [], caps: [], queue_new_targets: false, hl_markets: [],
    ...JSON.parse(text),
    public_key: req.public_key, label: name, expires_at_ms: now + days * 86_400_000, inactivity_ms: idle * 86_400_000,
  };
  const args = JSON.stringify({ key_id: req.key_id, grant });
  // 0.01 NEAR up to 2 KB of grant; past that, its bytes (JSON is longer than
  // the stored form, so this covers it).
  const deposit = args.length <= 2048 ? 10n ** 22n : BigInt(args.length + 256) * 10n ** 19n;
  status('');
  const msg = core.call_message(cfg.chain_id, wallet, nonce(), createdAt(), cfg.extension, 'grant_session', args, deposit.toString(), 20n);
  confirm(msg, {
    app: { name, url: req.app?.url ? String(req.app.url) : '' },
    after: async () => {
      // Tells the relayer, which reads the session from the extension
      // before telling the app anything.
      await post(`${RELAYER}/connect`, { wallet, key_id: req.key_id }).catch(() => {});
      if (returnUrl) {
        returnUrl.searchParams.set('wallet', wallet);
        returnUrl.searchParams.set('key_id', req.key_id);
        setTimeout(() => (location.href = returnUrl!.toString()), 1500);
        return `Approved. Taking you back to ${esc(returnUrl.host)}…`;
      }
      return `Approved. <b>${esc(name)}</b> can now act within these limits. You can revoke it here any time.`;
    },
  });
}

/** A decode failure blocks approval instead of showing something partial. */
let blocked = false;
function explainSafe(receiver: string, a: Described['external'][0]['actions'][0], ctx: SignCtx = {}) {
  try {
    return explain(receiver, a, ctx);
  } catch (e) {
    blocked = true;
    return `<span class="error"><b>Don't approve:</b> ${esc((e as Error).message ?? e)}</span>`;
  }
}

type Described = {
  chain_id: string;
  signer_id: string;
  deadline: string;
  internal: { op: string; account_id?: string; enable?: boolean }[];
  external: { receiver_id: string; actions: { kind: string; method?: string; args?: any; deposit_yocto?: string; gas_tgas?: number; amount_yocto?: string }[] }[];
};

/** Plain words for a call, worked out from the decoded arguments. */
function explain(receiver: string, a: Described['external'][0]['actions'][0], ctx: SignCtx = {}): string {
  const { tx, solMessage, btcPlan, tronRaw, xrpTx, tonBoc, stellarTx, legacyPlan, adaPlan, hlTransfer } = ctx;
  if (a.kind === 'transfer') return `<b>Approve:</b> Send <b>${formatNear(a.amount_yocto!)} NEAR</b> to ${receiver.length > 32 ? copyable(receiver) : copyableName(receiver)} on NEAR`;
  if (a.kind !== 'function_call') return `<span class="error">Unrecognised action: ${esc(JSON.stringify(a))}</span>`;
  const ours = receiver === cfg.extension;
  if (ours && a.method === 'grant_session') {
    const g = a.args.grant;
    // Every part of a grant must be shown; a part this page can't explain
    // blocks approval (explainSafe turns the throw into a warning).
    const KNOWN = ['public_key', 'label', 'expires_at_ms', 'inactivity_ms', 'near_venues', 'evm_venues', 'near_recipients', 'evm_recipients', 'caps', 'queue_new_targets', 'hl_markets', 'evm_self', 'evm_nft_floor', 'utxo', 'sol', 'xrpl', 'stellar', 'tron', 'ton'];
    const unknown = Object.keys(g).filter((k) => !KNOWN.includes(k));
    if (unknown.length) throw new Error(`This grant has parts this page can't show you (${unknown.join(', ')})`);
    const list = (xs: string[], none: string) => (xs?.length ? xs.map((x) => copyable(String(x))).join(', ') : none);
    const sections: string[] = [];
    if (g.evm_nft_floor && g.evm_nft_floor !== '0') sections.push(`<li>May list your NFTs on Seaport (OpenSea's contract) for no less than <b>${esc(fmtUnits(BigInt(g.evm_nft_floor), 18))}</b> in the chain's coin, to you</li>`);
    if (g.sol) {
      const venues = (g.sol.venues ?? []).map((v: any) => `${esc(SOL_PROGRAMS[v.program] ?? short(v.program))}${v.scope === v.program ? '' : ` (${esc(short(v.scope))})`}`);
      sections.push(`<li>On <b>Solana</b> (${copyable(g.sol.self_address)}): may use ${venues.length ? venues.join(', ') : 'no protocols'}; may pay ${list(g.sol.recipients, 'no one directly')}${g.sol.nft_floor && g.sol.nft_floor !== '0' ? `; may list NFTs on Tensor for no less than <b>${esc(fmtUnits(BigInt(g.sol.nft_floor), 9))} SOL</b>` : ''}${g.sol.perps?.length ? `; may ${perpsWords(g.sol.perps)}` : ''}. What protocols return comes back to that address.</li>`);
    }
    if (g.ton) sections.push(`<li>On <b>TON</b> (${copyable(g.ton.self_address)}): may pay ${list(g.ton.recipients, 'no one directly')}${g.ton.jettons?.length ? `; tokens ${g.ton.jettons.map((j: any) => copyable(j.master)).join(', ')}` : ''}${g.ton.evaa?.length ? `; may lend and borrow on EVAA (${g.ton.evaa.map((x: string) => copyable(x)).join(', ')})` : ''}${g.ton.nft ? '; may move your NFTs to the addresses listed' : ''}${g.ton.lst?.length ? `; may stake and unstake with Tonstakers (${g.ton.lst.map((l: any) => copyable(l.pool)).join(', ')}): tsTON and TON come back only to this address` : ''}</li>`);
    if (g.tron) sections.push(`<li>On <b>TRON</b> (${copyable(g.tron.self_address)}): may call ${(g.tron.venues ?? []).map((v: any) => `${esc(fnName(v.selector))} on ${TRON_NAMES[v.contract] ? `<b>${esc(TRON_NAMES[v.contract])}</b>` : copyable(v.contract)}`).join('; ') || 'nothing'}; may pay ${list(g.tron.recipients, 'no one directly')}${g.tron.nfts?.length ? `; may trade NFTs of ${list(g.tron.nfts, '')}` : ''}</li>`);
    if (g.xrpl) sections.push(`<li>On the <b>XRP Ledger</b> (${copyable(g.xrpl.self_address)}): ${[g.xrpl.dex && 'trade on its order book', g.xrpl.amm && 'use its AMM pools', g.xrpl.nft && 'trade NFTs'].filter(Boolean).join(', ') || 'payments only'}; assets ${list(g.xrpl.assets, 'XRP only')}; may pay ${list(g.xrpl.recipients, 'no one directly')}</li>`);
    if (g.stellar) sections.push(`<li>On <b>Stellar</b> (${copyable(g.stellar.self_address)}): ${[g.stellar.dex && 'trade on its order book', g.stellar.amm && 'use its AMM pools', g.stellar.blend?.length && `lend and borrow on Blend (${g.stellar.blend.map((x: string) => copyable(x)).join(', ')})`].filter(Boolean).join(', ') || 'payments only'}; assets ${list(g.stellar.assets, 'XLM only')}; may pay ${list(g.stellar.recipients, 'no one directly')}</li>`);
    const venues = [
      ...g.evm_venues.map((v: any) => `${esc(fnName(v.selector))} on ${POOLS[`${v.chain_id}:${String(v.to).toLowerCase()}`] ? `<b>${esc(POOLS[`${v.chain_id}:${String(v.to).toLowerCase()}`])}</b>` : esc(v.to)} (${esc(chainName(v.chain_id))})`),
      ...g.near_venues.map((v: any) => `call ${esc(v.method)} on ${esc(v.receiver_id)}`),
    ];
    const pays = [...g.evm_recipients, ...g.near_recipients].map(esc);
    return `${grantRequest(g, (ctx as any).app?.name ?? g.label, !!(ctx as any).app)}
      <details class="more"><summary>Details</summary>
      <p class="small"><b>Session key for "${esc(g.label)}"</b> (${esc(a.args.key_id)})</p>
      <ul class="small">
        <li>Agent key <span class="mono">${esc(g.public_key.slice(0, 16))}…</span></li>
        <li>May ${venues.length ? venues.join('; ') : 'call nothing'}</li>
        <li>May pay ${pays.length ? pays.join(', ') : 'no one directly'}</li>
        ${g.evm_self ? `<li>Results of those calls go only to your account ${copyable(g.evm_self)}</li>` : ''}
        ${(g.utxo ?? []).map((u: any) => `<li>May pay on <b>${esc(PAY_NAMES[u.chain] ?? u.chain)}</b>: ${u.recipients.length ? u.recipients.map((r: string) => copyable(utxoAddr(u.chain, r))).join(', ') : 'no one directly'}; change only to ${copyable(utxoAddr(u.chain, u.self_script))}</li>`).join('')}
        ${(g.hl_markets ?? []).map((m: any) => `<li>May trade <b>${esc(hlName(m.asset))}</b> on Hyperliquid testnet, at most <b>${esc(m.max_leverage)}x</b> leverage, signed by your account's EVM address</li>`).join('')}
        ${sections.join('')}
        <li>At most ${g.caps.length ? g.caps.map((c: any) => capWords(c.asset, c.daily)).join(', ') : 'nothing'} a day</li>
        <li>Until ${esc(new Date(g.expires_at_ms).toLocaleString())}, or after ${esc(Math.round(g.inactivity_ms / 86_400_000))} days unused</li>
        <li>${g.queue_new_targets ? '<span class="warn">May propose new recipients and contracts: each waits the delay, and you can cancel it</span>' : 'May not propose anything new'}</li>
      </ul></details>`;
  }
  if (receiver === cfg.signer && a.method === 'sign') {
    // The wallet asks the MPC network to sign for its EVM address. Show the
    // transaction, and only if the hash being signed is that transaction's.
    const req = a.args?.request;
    if (req?.path === 'tron') {
      if (!tronRaw || req.domain_id !== 0) throw new Error('A TRON signature request with no transaction to show');
      const d = JSON.parse(core.describe_tron_raw(tronRaw));
      if (req.payload_v2?.Ecdsa !== d.txid) throw new Error('The signature request does not match the transaction shown');
      return `<b>Approve:</b> Send <b>${esc(fmtUnits(BigInt(d.sun), 6))} TRX</b> to ${copyable(d.to)} on TRON`;
    }
    if (req?.path === 'xrp') {
      if (!xrpTx || req.domain_id !== 0) throw new Error('An XRP signature request with no transaction to show');
      const d = JSON.parse(core.describe_xrp_tx(xrpTx));
      if (req.payload_v2?.Ecdsa !== d.signing_hash) throw new Error('The signature request does not match the transaction shown');
      const tag = d.destination_tag != null ? ` · destination tag ${esc(d.destination_tag)}` : '';
      return `<b>Approve:</b> Send <b>${esc(fmtUnits(BigInt(d.drops), 6))} XRP</b> to ${copyable(d.to)} on XRP<div class="small muted">Network fee ${esc(fmtUnits(BigInt(d.fee_drops), 6, 6))} XRP${tag}</div>`;
    }
    if (req?.path === 'stellar') {
      // Stellar: the key signs the transaction hash; show the transaction,
      // and only if the hash requested is its hash.
      if (!stellarTx || req.domain_id !== 1) throw new Error('A Stellar signature request with no transaction to show');
      const d = JSON.parse(core.describe_stellar(stellarTx));
      if (req.payload_v2?.Eddsa !== d.hash) throw new Error('The signature request does not match the payment shown');
      const memo = d.memo ? ` · memo "${esc(d.memo)}"` : '';
      const create = d.kind === 'stellar_create_account' ? ' (a new account: this send creates it)' : '';
      return `<b>Approve:</b> Send <b>${esc(fmtUnits(BigInt(d.stroops), 7))} XLM</b> to ${copyable(d.to)} on Stellar${create}<div class="small muted">Network fee ${esc(fmtUnits(BigInt(d.fee_stroops), 7, 7))} XLM${memo}</div>`;
    }
    if (req?.path === 'ton') {
      // TON: the key signs the wallet message's hash; show the message, and
      // only if the hash requested is its hash.
      if (!tonBoc || req.domain_id !== 1) throw new Error('A TON signature request with no message to show');
      const d = JSON.parse(core.describe_ton(tonBoc));
      if (req.payload_v2?.Eddsa !== d.signing_hash) throw new Error('The signature request does not match the payment shown');
      const note = d.comment ? ` · comment "${esc(d.comment)}"` : '';
      return `<b>Approve:</b> Send <b>${esc(fmtUnits(BigInt(d.nanotons), 9))} GRAM</b> to ${copyable(d.to)} on TON<div class="small muted">Network fee paid from the balance${note}</div>`;
    }
    if (req?.path === 'bitcoin' || req?.path === 'litecoin') {
      // Bitcoin and Litecoin: each input needs its own signature. Confirm
      // checks the set of hashes; here each must be one of the plan's, on
      // the plan's own chain.
      if (!btcPlan || req.domain_id !== 0) throw new Error('A signature request with no transaction to show');
      const d = JSON.parse(core.describe_bitcoin_plan(btcPlan));
      if (d.chain !== req.path) throw new Error('The signature request is for another chain');
      const c = UTXO_SEND[d.chain];
      const k = d.sighashes.indexOf(req.payload_v2?.Ecdsa);
      if (k < 0) throw new Error('The signature request does not match the transaction shown');
      const pays = d.pays.map((x: any) => `<b>${esc(fmtUnits(BigInt(x.sat), 8, 8))} ${c.sym}</b> to ${copyable(x.to)}`).join(', ');
      return `<b>Approve:</b> Send ${pays} on ${esc(c.name)}<div class="small muted">Network fee ${esc(fmtUnits(BigInt(d.fee_sat), 8, 8))} ${c.sym} · signature ${k + 1} of ${d.sighashes.length}</div>`;
    }
    if (req?.path in LEGACY_SEND) {
      // Dogecoin, Bitcoin Cash, Zcash: as for Bitcoin, one signature per
      // input, each one of the plan's hashes, on the plan's own chain.
      if (!legacyPlan || req.domain_id !== 0) throw new Error('A signature request with no transaction to show');
      const d = JSON.parse(core.describe_legacy_plan(legacyPlan));
      if (d.chain !== req.path) throw new Error('The signature request is for another chain');
      const c = LEGACY_SEND[d.chain];
      const k = d.sighashes.indexOf(req.payload_v2?.Ecdsa);
      if (k < 0) throw new Error('The signature request does not match the transaction shown');
      const pays = d.pays.map((x: any) => `<b>${esc(fmtUnits(BigInt(x.sat), 8, 8))} ${c.sym}</b> to ${copyable(x.to)}`).join(', ');
      return `<b>Approve:</b> Send ${pays} on ${esc(c.name)}<div class="small muted">Network fee ${esc(fmtUnits(BigInt(d.fee_sat), 8, 8))} ${c.sym} · signature ${k + 1} of ${d.sighashes.length}</div>`;
    }
    if (req?.path === 'cardano') {
      // Cardano: the key signs the body's hash; show the body, and only if
      // the hash requested is its hash.
      if (!adaPlan || req.domain_id !== 1) throw new Error('A Cardano signature request with no transaction to show');
      const d = JSON.parse(core.describe_cardano_plan(adaPlan));
      if (req.payload_v2?.Eddsa !== d.hash) throw new Error('The signature request does not match the transaction shown');
      const pays = d.pays.map((x: any) => `<b>${esc(fmtUnits(BigInt(x.lovelace), 6, 6))} ADA</b> to ${copyable(x.to)}`).join(', ');
      return `<b>Approve:</b> Send ${pays} on Cardano<div class="small muted">Network fee ${esc(fmtUnits(BigInt(d.fee_lovelace), 6, 6))} ADA</div>`;
    }
    if (req?.payload_v2?.Eddsa !== undefined) {
      // Solana: show the transfer, and only if the bytes being signed are it.
      if (!solMessage || req.path !== 'solana' || req.domain_id !== 1 || req.payload_v2.Eddsa !== solMessage) {
        throw new Error('The signature request does not match the transfer shown');
      }
      const d = JSON.parse(core.describe_solana_message(solMessage));
      return `<b>Approve:</b> Send <b>${esc(fmtUnits(BigInt(d.lamports), 9))} SOL</b> to ${copyable(d.to)} on Solana`;
    }
    if (hlTransfer) {
      // Hyperliquid spot↔perps: the digest is rebuilt here from the fields.
      const h = core.hl_transfer_hash(hlTransfer.amount, hlTransfer.to_perp, BigInt(hlTransfer.nonce));
      if (req?.path !== 'evm' || req.domain_id !== 0 || req.payload_v2?.Ecdsa !== h) throw new Error('The signature request does not match the transfer shown');
      return `<b>Approve:</b> Move <b>${esc(hlTransfer.amount)} USDC</b> from ${hlTransfer.to_perp ? 'spot to perps' : 'perps to spot'} on Hyperliquid testnet<div class="small muted">Within your own Hyperliquid account; nothing leaves it.</div>`;
    }
    if (!tx) return `<span class="error">A signature request with no transaction to show. Don't approve.</span>`;
    const d: EvmDescribed = JSON.parse(core.describe_evm_tx(tx));
    if (req?.path !== 'evm' || req?.payload_v2?.Ecdsa !== d.hash || req?.domain_id !== 0) {
      throw new Error('The signature request does not match the transaction shown');
    }
    return `<b>Approve:</b> ${explainEvm(d)}<div class="small muted">Gas limit ${esc(JSON.parse(tx).gas_limit)} · EVM nonce ${esc(JSON.parse(tx).nonce)}</div>`;
  }
  if (cfg.names && receiver === cfg.names && a.method === 'register' && a.deposit_yocto === '0') {
    return `<b>Take the name ${copyableName(`${a.args?.name}.${cfg.names}`)}</b> for this account. NEAR and tokens sent to it are forwarded here. It can't be changed later.`;
  }
  if (ours && a.method === 'cancel_queued') return `<b>Cancel queued request #${esc(a.args.id)}</b>. It will never run.`;
  if (ours && a.method === 'revoke_session') return `<b>Revoke the session key ${esc(a.args.key_id)}</b>. Its next request is refused.`;
  return `<span class="warn">Call ${esc(a.method)} on ${esc(receiver)}</span>${ours ? '' : ' <b class="error">(not the Marvii extension)</b>'}`;
}

/** What a Confirm sheet decodes alongside the message: the transaction each
 *  signature request must match. */
type SignCtx = { tx?: string; solMessage?: string; btcPlan?: string; tronRaw?: string; xrpTx?: string; tonBoc?: string; stellarTx?: string; legacyPlan?: string; adaPlan?: string; hlTransfer?: { amount: string; to_perp: boolean; nonce: number } };

function confirm(msgJson: string, ctx: SignCtx & { pendingId?: number; app?: { name: string; url: string }; after?: (out: Outcome & { mpc_signature?: unknown }) => Promise<string> } = {}) {
  const msg = core.normalize_request(msgJson);
  const d: Described = JSON.parse(core.describe_message(msg));
  blocked = false;
  if (ctx.btcPlan) {
    // Exactly one signature per input of the transaction shown, no more.
    const asked = d.external.flatMap((p) => p.actions.map((a) => a.args?.request?.payload_v2?.Ecdsa)).sort();
    const want = [...JSON.parse(core.describe_bitcoin_plan(ctx.btcPlan)).sighashes].sort();
    if (JSON.stringify(asked) !== JSON.stringify(want)) blocked = true;
  }
  if (ctx.legacyPlan) {
    const asked = d.external.flatMap((p) => p.actions.map((a) => a.args?.request?.payload_v2?.Ecdsa)).sort();
    const want = [...JSON.parse(core.describe_legacy_plan(ctx.legacyPlan)).sighashes].sort();
    if (JSON.stringify(asked) !== JSON.stringify(want)) blocked = true;
  }
  const m = document.getElementById('modal')!;
  const known = cfg.known_extensions ?? [];
  const ops = d.internal.map((o) => {
    if (o.op === 'add_extension' && o.account_id === cfg.extension) return `<li><b>Move to the current Marvii extension</b> <span class="mono small">${esc(short(o.account_id))}</span>. Your account keeps its address and funds.</li>`;
    if (o.op === 'remove_extension' && o.account_id && known.includes(o.account_id) && o.account_id !== cfg.extension) return `<li><b>Remove the older Marvii extension</b> <span class="mono small">${esc(short(o.account_id))}</span>. Agents added under it stop working; add them again.</li>`;
    return `<li class="error"><b>Changes the wallet itself:</b> ${esc(o.op)} ${esc(o.account_id ?? o.enable ?? '')}</li>`;
  });
  const calls = d.external.flatMap((p) =>
    p.actions.map(
      (a) => `<li>${explainSafe(p.receiver_id, a, ctx)}
        <details class="more"><summary>Technical details</summary>
          <div class="small muted">${a.deposit_yocto && a.deposit_yocto !== '0' ? `Attaches ${formatNear(a.deposit_yocto)} NEAR · ` : ''}${a.gas_tgas ? `${a.gas_tgas} TGas` : ''}</div>
          ${a.args === undefined ? '' : `<pre class="mono small">${esc(JSON.stringify(a.args, null, 2))}</pre>`}
        </details></li>`,
    ),
  );
  const lone = d.internal.length === 0 && d.external.length === 1 && d.external[0].actions.length === 1;
  const isRequest = lone && d.external[0].receiver_id === cfg.extension && d.external[0].actions[0].method === 'grant_session';
  // A request: one "Details" holding everything, the technical part included.
  const reqCall = () => {
    const a = d.external[0].actions[0];
    const tech = `<p class="small muted">${a.deposit_yocto && a.deposit_yocto !== '0' ? `Attaches ${formatNear(a.deposit_yocto)} NEAR · ` : ''}${a.gas_tgas ? `${a.gas_tgas} TGas` : ''}</p><pre class="mono small">${esc(JSON.stringify(a.args, null, 2))}</pre>`;
    const html = explainSafe(d.external[0].receiver_id, a, ctx);
    return html.endsWith('</ul></details>') ? html.slice(0, -'</details>'.length) + tech + '</details>' : html;
  };
  m.innerHTML = isRequest
    ? `
    <div class="backdrop"><div class="card sheet request">
      <ul class="list bare"><li>${reqCall()}</li></ul>
      <div class="req-actions"><button class="primary big" id="yes">Approve with passkey</button><button class="ghost" id="no">Not now</button></div>
      <p class="small muted center">Valid until ${esc(new Date(d.deadline).toLocaleTimeString())}. Decoded on this page from the exact message your passkey signs.</p>
      <div id="status" class="status"></div>
    </div></div>`
    : `
    <div class="backdrop"><div class="card sheet">
      ${ctx.app
        ? `<h2><b>${esc(ctx.app.name)}</b> wants access to your account</h2>
           <p class="small muted">${ctx.app.url ? `${esc(ctx.app.url)} · ` : ''}The name is the app's own claim: approve only if you started this.</p>
           <p class="small tee">✓ Its key was made inside Marvii's TEE, checked on NEAR. No one, not even the app, can read it.</p>`
        : `<h2>Confirm</h2><p class="small muted">Your account will:</p>`}
      <ul class="list">${[...ops, ...calls].join('') || '<li>Nothing</li>'}</ul>
      <p class="small muted">Valid until ${esc(new Date(d.deadline).toLocaleString())}. Decoded on this page from the exact message your passkey signs.</p>
      <div class="row end"><button class="secondary" id="no">Reject</button><button class="primary" id="yes">Approve with passkey</button></div>
      <div id="status" class="status"></div>
    </div></div>`;
  if (blocked) (document.getElementById('yes') as HTMLButtonElement).disabled = true;
  document.getElementById('no')!.onclick = () => (m.innerHTML = '');
  document.getElementById('yes')!.onclick = async () => {
    let sent = false;
    try {
      busy('Waiting for your passkey…');
      const relay = signAndRelay(msg, ctx.pendingId);
      relay.then(() => busy('Confirming…')).catch(() => {});
      const out = await within(relay, 120000);
      sent = true;
      if (!out.ok) throw new Error('Refused on-chain: ' + JSON.stringify(out.failures));
      busy(ctx.after ? 'Sending…' : 'Confirming…');
      const done = ctx.after ? await within(ctx.after(out), 90000) : `Done. Transaction ${esc(out.tx_hash)}`;
      m.innerHTML = '';
      busy('Updating your wallet…');
      await backToWallet();
      busy(null);
      const el = document.getElementById('status');
      if (el) {
        el.className = 'status ok';
        el.innerHTML = done;
      }
    } catch (e) {
      busy(null);
      const msg = String((e as Error).message ?? e);
      if (msg === 'timeout') {
        // It may still land: go back to the wallet, where balances update.
        m.innerHTML = '';
        await backToWallet();
        status(sent ? 'Approved, still confirming. Your balances update when it lands.' : 'This is taking longer than usual. Check your balances in a minute.', 'info');
      } else {
        status(msg, 'error');
      }
    }
  };
}

main().catch((e) => render(`<section class="card"><h1>Marvii</h1><p class="error">${esc(e.message ?? e)}</p><p class="muted small">Is the relayer running at the configured address?</p></section>`));
