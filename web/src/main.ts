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
};
/** Hyperliquid testnet perps by asset index (`meta` universe, checked
 *  9 November 2026). */
const HL_MARKETS: Record<number, string> = { 0: 'SOL', 3: 'BTC', 4: 'ETH', 54: 'NEAR' };
const hlName = (a: number) => (HL_MARKETS[a] ? `${HL_MARKETS[a]} perps` : `market #${a}`);
/** Cap amounts in plain words where the page knows the asset. */
/** The payment-only chains agents may pay on (Dogecoin's signatures don't
 *  commit to amounts, so its fees can't be capped: not offered). */
const PAY_CHAINS: Record<string, string> = { bitcoin: 'BTC', litecoin: 'LTC', 'bitcoin-cash': 'BCH', zcash: 'ZEC' };
const PAY_NAMES: Record<string, string> = { bitcoin: 'Bitcoin', litecoin: 'Litecoin', 'bitcoin-cash': 'Bitcoin Cash', zcash: 'Zcash' };
function capWords(asset: string, daily: string): string {
  if (asset === 'hyperliquid:usd') return `$${esc(fmtUnits(BigInt(daily), 6, 2))} of Hyperliquid orders`;
  if (asset.startsWith('utxo:') && PAY_CHAINS[asset.slice(5)]) return `${esc(fmtUnits(BigInt(daily), 8, 8))} ${PAY_CHAINS[asset.slice(5)]} (fees included)`;
  return `${esc(daily)} of ${esc(asset)}`;
}
const POOLS: Record<string, string> = {
  '84532:0x8bab6d1b75f19e9ed9fce8b9bd338844ff79ae27': 'Aave V3 pool',
  '84532:0x94cc0aac535ccdb3c01d6787d6413c739ae12bc4': 'Uniswap V3 router',
};
/** Venue functions by selector (keccak256 of the signature). The extension
 *  checks where each one's result goes (v5). */
const FN_NAMES: Record<string, string> = { '0x617ba037': 'supply', '0x69328dec': 'withdraw', '0x573ade81': 'repay', '0x04e45aaf': 'swap' };
/** Venues the grant form offers by name. Aave V3 from bgd-labs/aave-address-book
 *  (checked on-chain); Uniswap V3 SwapRouter02 on Base Sepolia, whose
 *  factory() and WETH9() were checked on-chain. */
const VENUES = [
  { id: 'aave', name: 'Aave V3', what: 'supply, withdraw, repay', chain: 84532, to: '0x8bab6d1b75f19e9ed9fce8b9bd338844ff79ae27', selectors: ['0x617ba037', '0x69328dec', '0x573ade81'] },
  { id: 'uniswap', name: 'Uniswap V3', what: 'swap', chain: 84532, to: '0x94cc0aac535ccdb3c01d6787d6413c739ae12bc4', selectors: ['0x04e45aaf'] },
];
const fnName = (sel: string) => FN_NAMES[sel.toLowerCase()] ?? `function ${sel}`;
const chainName = (id: number) => EVM_CHAINS.find((c) => c.id === id)?.name ?? `chain ${id}`;
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
async function backToWallet() {
  try {
    await within(dashboard(), 30000);
  } catch {
    location.reload();
  }
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

const VIEW_TITLES: Record<string, string> = { home: 'Marvii', assets: 'Assets', swap: 'Swap', agents: 'Apps & agents' };
/** Shows the view the URL's hash names (#/assets, #/chain-84532, …). */
function route() {
  const want = location.hash.replace(/^#\/?/, '') || 'home';
  const views = Array.from(document.querySelectorAll<HTMLElement>('.view'));
  const v = views.find((x) => x.dataset.view === want) ?? views.find((x) => x.dataset.view === 'home');
  views.forEach((x) => x.classList.toggle('on', x === v));
  document.title = VIEW_TITLES[v?.dataset.view ?? ''] ? `${VIEW_TITLES[v!.dataset.view!]} · Marvii` : 'Marvii';
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
        <a class="tile" href="#/swap"><span class="tile-ic">${ICON_SWAP}</span><b>Swap</b><span class="small muted">NEAR Intents</span></a>
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
        <p class="muted small">Live prices from NEAR Intents' solvers, between balances held in NEAR Intents by this account. Quote only: Intents runs on mainnet, and this testnet wallet never deposits.</p>
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
        <div class="row between"><h1>Apps &amp; agents</h1><button id="add" class="primary">Add</button></div>
        <p class="muted small">Each gets a key that NEAR checks on every request: which venues, how much a day, until when. Revoke any of them instantly.</p>
        <ul id="sessions" class="list"><li class="muted">Loading…</li></ul>
      </section>
    </section>
    <div id="modal"></div>`);
  route();
  document.getElementById('add')!.onclick = grantForm;
  swapCard();
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
      k === 'near' ? nearSendForm() : k === 'solana' ? solSendForm() : k === 'bitcoin' || k === 'litecoin' ? btcSendForm(k) : k === 'tron' ? tronSendForm() : k === 'xrp' ? xrpSendForm() : k === 'cardano' ? adaSendForm() : k in LEGACY_SEND ? legacySendForm(k) : sendForm(Number(k));
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
        if (c.id in LEGACY_SEND) legacyAccounts[c.id] = { address: addr, balance: bal };
        if (c.id === 'cardano') ada = { address: addr, balance: bal, key: k };
        const canSend = ['solana', 'bitcoin', 'litecoin', 'tron', 'xrp', 'cardano', ...Object.keys(LEGACY_SEND)].includes(c.id);
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
  await Promise.all([loadSessions(), loadPending(), loadQueued()]);
}

type QueuedItem = { wallet: string; key_id: string; action: string; created_ms: number; ready_at_ms: number };

/** Plain words for a queued action, decoded on this page. */
function explainAction(json: string): string {
  const a = JSON.parse(json);
  if (a.type === 'evm_tx') return explainEvm(JSON.parse(core.describe_evm_tx(JSON.stringify(a.tx))));
  if (a.type === 'near_call') return `Call ${esc(a.method)} on ${esc(a.receiver_id)} with ${formatNear(a.deposit)} NEAR`;
  if (a.type === 'utxo_pay' && PAY_CHAINS[a.chain]) {
    const pays = a.outputs.map((o: any) => `${esc(fmtUnits(BigInt(o.value), 8, 8))} ${PAY_CHAINS[a.chain]} to ${copyable(core.utxo_address(a.chain, o.script))}`).join(', ');
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

async function loadSessions() {
  const ul = document.getElementById('sessions')!;
  // Read from the extension's index: what the chain says, not this browser.
  const rows = await view<[string, Session][]>(cfg.extension, 'list_sessions', { wallet }).catch(() => [] as [string, Session][]);
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
      const caps = s.grant.caps.map((c) => {
        const [kind, chain, token] = c.asset.split(':');
        if (kind === 'evm' && token) return `${amount(Number(chain), token, c.daily)} a day on ${esc(chainName(Number(chain)))}`;
        return `${capWords(c.asset, c.daily)} a day`;
      }).join(', ') || 'no spending';
      const venues = [
        ...s.grant.evm_venues.map((v) => `${esc(fnName(v.selector))} on ${named(v.chain_id, v.to)}, ${esc(chainName(v.chain_id))}`),
        ...s.grant.near_venues.map((v) => `${esc(v.receiver_id)}.${esc(v.method)}`),
        ...(s.grant.hl_markets ?? []).map((m) => `Hyperliquid ${esc(hlName(m.asset))} up to ${esc(m.max_leverage)}x`),
        ...((s.grant as any).utxo ?? []).map((u: any) => `payments on ${esc(PAY_NAMES[u.chain] ?? u.chain)} to ${u.recipients.map((r: string) => esc(short(core.utxo_address(u.chain, r)))).join(', ') || 'no one'}`),
      ].join('; ') || 'none';
      return `<li class="session">
        <div class="row between"><b>${esc(s.grant.label)}</b> ${state}</div>
        <div class="small muted">key ${esc(k)} · ${s.nonce} requests · until ${esc(new Date(s.grant.expires_at_ms).toLocaleString())}</div>
        <div class="small">Venues: ${venues}</div>
        <div class="small">Caps: ${caps}</div>
        ${s.revoked ? '' : `<button class="danger small-btn" data-revoke="${esc(k)}">Revoke</button>`}
      </li>`;
    })
    .join('');
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
  (document.getElementById('grant') as HTMLFormElement).onsubmit = (ev) => {
    ev.preventDefault();
    const f = new FormData(ev.target as HTMLFormElement);
    const g = (k: string) => String(f.get(k) ?? '').trim();
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
  const { tx, solMessage, btcPlan, tronRaw, xrpTx, legacyPlan, adaPlan, hlTransfer } = ctx;
  if (a.kind === 'transfer') return `<b>Approve:</b> Send <b>${formatNear(a.amount_yocto!)} NEAR</b> to ${receiver.length > 32 ? copyable(receiver) : copyableName(receiver)} on NEAR`;
  if (a.kind !== 'function_call') return `<span class="error">Unrecognised action: ${esc(JSON.stringify(a))}</span>`;
  const ours = receiver === cfg.extension;
  if (ours && a.method === 'grant_session') {
    const g = a.args.grant;
    const venues = [
      ...g.evm_venues.map((v: any) => `${esc(fnName(v.selector))} on ${POOLS[`${v.chain_id}:${String(v.to).toLowerCase()}`] ? `<b>${esc(POOLS[`${v.chain_id}:${String(v.to).toLowerCase()}`])}</b>` : esc(v.to)} (${esc(chainName(v.chain_id))})`),
      ...g.near_venues.map((v: any) => `call ${esc(v.method)} on ${esc(v.receiver_id)}`),
    ];
    const pays = [...g.evm_recipients, ...g.near_recipients].map(esc);
    return `<b>Give "${esc(g.label)}" a session key</b> (${esc(a.args.key_id)})
      <ul>
        <li>Agent key <span class="mono">${esc(g.public_key.slice(0, 16))}…</span></li>
        <li>May ${venues.length ? venues.join('; ') : 'call nothing'}</li>
        <li>May pay ${pays.length ? pays.join(', ') : 'no one directly'}</li>
        ${g.evm_self ? `<li>Results of those calls go only to your account ${copyable(g.evm_self)}</li>` : ''}
        ${(g.utxo ?? []).map((u: any) => `<li>May pay on <b>${esc(PAY_NAMES[u.chain] ?? u.chain)}</b>: ${u.recipients.length ? u.recipients.map((r: string) => copyable(core.utxo_address(u.chain, r))).join(', ') : 'no one directly'}; change only to ${copyable(core.utxo_address(u.chain, u.self_script))}</li>`).join('')}
        ${(g.hl_markets ?? []).map((m: any) => `<li>May trade <b>${esc(hlName(m.asset))}</b> on Hyperliquid testnet, at most <b>${esc(m.max_leverage)}x</b> leverage, signed by your account's EVM address</li>`).join('')}
        <li>At most ${g.caps.map((c: any) => capWords(c.asset, c.daily)).join(', ')} a day</li>
        <li>Until ${esc(new Date(g.expires_at_ms).toLocaleString())}, or after ${esc(Math.round(g.inactivity_ms / 86_400_000))} days unused</li>
        <li>${g.queue_new_targets ? '<span class="warn">May propose new recipients and contracts: each waits the delay, and you can cancel it</span>' : 'May not propose anything new'}</li>
      </ul>`;
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
type SignCtx = { tx?: string; solMessage?: string; btcPlan?: string; tronRaw?: string; xrpTx?: string; legacyPlan?: string; adaPlan?: string; hlTransfer?: { amount: string; to_perp: boolean; nonce: number } };

function confirm(msgJson: string, ctx: SignCtx & { pendingId?: number; after?: (out: Outcome & { mpc_signature?: unknown }) => Promise<string> } = {}) {
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
        <div class="small muted">${a.deposit_yocto && a.deposit_yocto !== '0' ? `Attaches ${formatNear(a.deposit_yocto)} NEAR · ` : ''}${a.gas_tgas ? `${a.gas_tgas} TGas` : ''}</div>
        ${a.args === undefined ? '' : `<details><summary class="small">Exact arguments</summary><pre class="mono small">${esc(JSON.stringify(a.args, null, 2))}</pre></details>`}</li>`,
    ),
  );
  m.innerHTML = `
    <div class="backdrop"><div class="card sheet">
      <h2>Confirm</h2>
      <p class="small muted">Your account will:</p>
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
