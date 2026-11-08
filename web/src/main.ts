import '@fontsource-variable/geist';
import '@fontsource-variable/geist-mono';
import './style.css';
import init, * as core from './core/core.js';
import { createPasskey, signChallenge, type Passkey } from './passkey';
import { account, formatNear, relayer, setRpc, view, type Config } from './near';

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
const TOKEN_BALANCES = [
  { chain: 84532, token: '0xba50cd2a20f6da35d788639e581bca8d0b5d4d5f', symbol: 'USDC', decimals: 6 },
  { chain: 84532, token: '0x10f1a9d11cdf50041f3f8cb7191cbe2f31750acc', symbol: 'aUSDC', decimals: 6 },
];


// Tokens the page can name. Anything else is shown by address and raw units.
// Aave's test USDC on Base Sepolia (bgd-labs/aave-address-book, checked on-chain).
const TOKENS: Record<string, { symbol: string; decimals: number }> = {
  '84532:0xba50cd2a20f6da35d788639e581bca8d0b5d4d5f': { symbol: 'USDC', decimals: 6 },
};
const POOLS: Record<string, string> = { '84532:0x8bab6d1b75f19e9ed9fce8b9bd338844ff79ae27': 'Aave V3 pool' };
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

async function main() {
  await init();
  cfg = await relayer<Config>('/config');
  setRpc(cfg.rpc);
  // Read-only view of any wallet: ?wallet=0s… (no passkey, no buttons).
  const viewOnly = new URLSearchParams(location.search).get('wallet');
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
    await dashboard();
  } else {
    start();
  }
}

function start() {
  render(`
    <section class="card hero">
      <h1>One passkey, every chain</h1>
      <p>No seed phrase, no password. Give agents and apps limited, revocable permission to act for you, and approve anything outside it with one tap.</p>
      <button id="create" class="primary">Create with a passkey</button>
      <p class="muted small">Testnet only. Unaudited.</p>
      <div id="status" class="status"></div>
    </section>`);
  document.getElementById('create')!.onclick = async () => {
    try {
      status('Creating your passkey…');
      const p = await createPasskey(`Marvii ${new Date().toISOString().slice(0, 10)}`);
      pk = p;
      wallet = core.wallet_account_id(cfg.wallet_code_hash, p.publicKey, cfg.extension);
      store.passkey = { ...p, wallet } as Passkey;
      status('Activating your wallet: approve with your passkey once more…');
      const msg = core.empty_message(cfg.chain_id, wallet, nonce(), createdAt());
      const out = await signAndRelay(msg);
      if (!out.ok) throw new Error('Activation failed: ' + JSON.stringify(out.failures));
      await relayer('/wallet/fund', { public_key: p.publicKey, wallet }).catch(() => {});
      await dashboard();
    } catch (e) {
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
  return relayer<Outcome>('/wallet/relay', { public_key: pk!.publicKey, msg: JSON.parse(msg), proof, pending_id: pendingId });
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

const fmtUnits = (n: bigint, decimals: number, shown = 4) => {
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
  grant: { label: string; public_key: string; expires_at_ms: number; inactivity_ms: number; caps: { asset: string; daily: string }[]; evm_venues: { chain_id: number; to: string; selector: string }[]; near_venues: { receiver_id: string; method: string }[] };
  nonce: number;
  last_used_ms: number;
  revoked: boolean;
};

function chainRow(id: string, name: string) {
  return `<li><details id="${id}"><summary class="row between"><b>${esc(name)}</b><span class="muted bal">…</span></summary><div class="assets"></div></details></li>`;
}

/** Fills a chain row: the native balance on the row, and in its dropdown the
 *  address and every asset held there. */
function fillChain(id: string, address: string, native: string, assets: string[], explorer?: string, sendChain?: number) {
  const d = document.getElementById(id)!;
  d.querySelector('.bal')!.textContent = native;
  const link = explorer ? ` <a class="small" target="_blank" rel="noopener" href="${esc(explorer)}">Explorer</a>` : '';
  const send = sendChain ? `<div class="row end"><button class="secondary small-btn" data-send="${sendChain}">Send</button></div>` : '';
  d.querySelector('.assets')!.innerHTML =
    `<div class="row between small"><span class="muted">Address</span><span>${copyable(address)}${link}</span></div>` +
    [`<div class="row between"><span>${esc(native.split(' ').pop()!)}</span><span>${esc(native.split(' ')[0])}</span></div>`, ...assets].join('') +
    send;
}

/** What each EVM chain holds, for the Send form. */
type Held = { symbol: string; token: string; decimals: number; balance: string };
const holdings: Record<number, Held[]> = {};
let evmAddr = '';

const toUnits = (amount: string, decimals: number) => {
  const [w, f = ''] = amount.trim().split('.');
  if (!/^\d+$/.test(w || '0') || !/^\d*$/.test(f) || f.length > decimals) throw new Error('Enter an amount like 1.5');
  return (BigInt(w || '0') * 10n ** BigInt(decimals) + BigInt((f + '0'.repeat(decimals)).slice(0, decimals) || '0')).toString();
};

/** Sends from the account's EVM address on one chain: the passkey approves
 *  the exact transaction, then the page broadcasts it. */
function sendForm(chainId: number) {
  const c = EVM_CHAINS.find((x) => x.id === chainId)!;
  const assets = holdings[chainId] ?? [];
  const m = document.getElementById('modal')!;
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

async function dashboard() {
  render(`
    <section class="card">
      <div class="row between"><h1>Balances</h1><span class="pill">${esc(cfg.chain_id === 'mainnet' ? 'Mainnet' : 'Testnet')}</span></div>
      <ul class="list chains">${chainRow('chain-near', 'NEAR')}${EVM_CHAINS.map((c) => chainRow(`chain-${c.id}`, c.name)).join('')}</ul>
      <div id="status" class="status"></div>
    </section>
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
      <div class="row between"><h2>Agents and Apps</h2><button id="add" class="secondary">Add</button></div>
      <ul id="sessions" class="list"><li class="muted">Loading…</li></ul>
    </section>
    <div id="modal"></div>`);
  document.getElementById('add')!.onclick = grantForm;
  app.onclick = (e) => {
    const b = (e.target as HTMLElement).closest<HTMLElement>('[data-copy]');
    if (b) {
      navigator.clipboard?.writeText(b.dataset.copy!).catch(() => {});
      b.classList.add('copied');
      setTimeout(() => b.classList.remove('copied'), 1200);
    }
    const sb = (e.target as HTMLElement).closest<HTMLElement>('[data-send]');
    if (sb) sendForm(Number(sb.dataset.send));
  };

  account(wallet).then((a) => fillChain('chain-near', wallet, `${a ? formatNear(a.amount) : '0.0000'} NEAR`, []));
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
                return `<div class="row between"><span>${esc(t.symbol)}</span><span>${esc(b)}</span></div>`;
              }),
            );
            holdings[c.id] = held;
            const canSend = held.some((h) => Number(h.balance) > 0);
            fillChain(`chain-${c.id}`, addr, native, rows.filter(Boolean), c.explorer ? c.explorer + addr : undefined, canSend ? c.id : undefined);
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
    case 'native_transfer': return `Send ${esc(d.value)} wei to <span class="mono">${esc(d.to)}</span> ${on}`;
    default: return `<span class="error">Unrecognised call ${esc(c.selector)} to ${esc(c.contract)} ${on}</span>`;
  }
}

async function loadPending() {
  const items = (await relayer<Pending[]>(`/pending?wallet=${encodeURIComponent(wallet)}`).catch(() => [] as Pending[])).filter((p) => p.status === 'pending');
  const card = document.getElementById('pending-card')!;
  card.hidden = !items.length;
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
  ul.innerHTML = rows
    .map(([k, s]) => {
      const expired = now >= s.grant.expires_at_ms || now >= s.last_used_ms + s.grant.inactivity_ms;
      const state = s.revoked ? '<span class="pill bad">revoked</span>' : expired ? '<span class="pill bad">expired</span>' : '<span class="pill ok">active</span>';
      const caps = s.grant.caps.map((c) => {
        const [kind, chain, token] = c.asset.split(':');
        if (kind === 'evm' && token) return `${amount(Number(chain), token, c.daily)} a day on ${esc(chainName(Number(chain)))}`;
        return `${esc(c.daily)} of ${esc(c.asset)} a day`;
      }).join(', ') || 'no spending';
      const venues = [
        ...s.grant.evm_venues.map((v) => `${v.selector === '0x617ba037' ? 'supply on ' : `function ${esc(v.selector)} on `}${named(v.chain_id, v.to)}, ${esc(chainName(v.chain_id))}`),
        ...s.grant.near_venues.map((v) => `${esc(v.receiver_id)}.${esc(v.method)}`),
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
      <fieldset><legend>Allowed contract on an EVM chain</legend>
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
      evm_venues: g('to') && g('selector') ? [{ chain_id: chain, to: g('to').toLowerCase(), selector: g('selector').toLowerCase() }] : [],
      near_recipients: [],
      evm_recipients: [],
      caps: [{ asset: g('token') ? `evm:${chain}:${g('token').toLowerCase()}` : `evm:${chain}`, daily: g('daily') }],
      queue_new_targets: f.get('queue') === 'on',
    };
    const key_id = g('key_id');
    // 0.01 NEAR: the extension's storage deposit for a session.
    const msg = core.call_message(cfg.chain_id, wallet, nonce(), createdAt(), cfg.extension, 'grant_session', JSON.stringify({ key_id, grant }), '10000000000000000000000', 20n);
    confirm(msg);
  };
}

/** A decode failure blocks approval instead of showing something partial. */
let blocked = false;
function explainSafe(receiver: string, a: Described['external'][0]['actions'][0], tx?: string) {
  try {
    return explain(receiver, a, tx);
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
function explain(receiver: string, a: Described['external'][0]['actions'][0], tx?: string): string {
  if (a.kind === 'transfer') return `Send ${formatNear(a.amount_yocto!)} NEAR to ${esc(receiver)}`;
  if (a.kind !== 'function_call') return `<span class="error">Unrecognised action: ${esc(JSON.stringify(a))}</span>`;
  const ours = receiver === cfg.extension;
  if (ours && a.method === 'grant_session') {
    const g = a.args.grant;
    const venues = [
      ...g.evm_venues.map((v: any) => `call function ${esc(v.selector)} on ${esc(v.to)} (chain ${esc(v.chain_id)})`),
      ...g.near_venues.map((v: any) => `call ${esc(v.method)} on ${esc(v.receiver_id)}`),
    ];
    const pays = [...g.evm_recipients, ...g.near_recipients].map(esc);
    return `<b>Give "${esc(g.label)}" a session key</b> (${esc(a.args.key_id)})
      <ul>
        <li>Agent key <span class="mono">${esc(g.public_key.slice(0, 16))}…</span></li>
        <li>May ${venues.length ? venues.join('; ') : 'call nothing'}</li>
        <li>May pay ${pays.length ? pays.join(', ') : 'no one directly'}</li>
        <li>At most ${g.caps.map((c: any) => `${esc(c.daily)} of ${esc(c.asset)}`).join(', ')} a day</li>
        <li>Until ${esc(new Date(g.expires_at_ms).toLocaleString())}, or after ${esc(Math.round(g.inactivity_ms / 86_400_000))} days unused</li>
        <li>${g.queue_new_targets ? '<span class="warn">May propose new recipients and contracts: each waits the delay, and you can cancel it</span>' : 'May not propose anything new'}</li>
      </ul>`;
  }
  if (receiver === cfg.signer && a.method === 'sign') {
    // The wallet asks the MPC network to sign for its EVM address. Show the
    // transaction, and only if the hash being signed is that transaction's.
    const req = a.args?.request;
    if (!tx) return `<span class="error">A signature request with no transaction to show. Don't approve.</span>`;
    const d: EvmDescribed = JSON.parse(core.describe_evm_tx(tx));
    if (req?.path !== 'evm' || req?.payload_v2?.Ecdsa !== d.hash || req?.domain_id !== 0) {
      throw new Error('The signature request does not match the transaction shown');
    }
    return `<b>Approve:</b> ${explainEvm(d)}<div class="small muted">Gas limit ${esc(JSON.parse(tx).gas_limit)} · EVM nonce ${esc(JSON.parse(tx).nonce)}</div>`;
  }
  if (ours && a.method === 'cancel_queued') return `<b>Cancel queued request #${esc(a.args.id)}</b>. It will never run.`;
  if (ours && a.method === 'revoke_session') return `<b>Revoke the session key ${esc(a.args.key_id)}</b>. Its next request is refused.`;
  return `<span class="warn">Call ${esc(a.method)} on ${esc(receiver)}</span>${ours ? '' : ' <b class="error">(not the Marvii extension)</b>'}`;
}

function confirm(msgJson: string, ctx: { pendingId?: number; tx?: string; after?: (out: Outcome & { mpc_signature?: unknown }) => Promise<string> } = {}) {
  const msg = core.normalize_request(msgJson);
  const d: Described = JSON.parse(core.describe_message(msg));
  blocked = false;
  const m = document.getElementById('modal')!;
  const ops = d.internal.map((o) => `<li class="error"><b>Changes the wallet itself:</b> ${esc(o.op)} ${esc(o.account_id ?? o.enable ?? '')}</li>`);
  const calls = d.external.flatMap((p) =>
    p.actions.map(
      (a) => `<li>${explainSafe(p.receiver_id, a, ctx.tx)}
        <div class="small muted">${a.deposit_yocto && a.deposit_yocto !== '0' ? `Attaches ${formatNear(a.deposit_yocto)} NEAR · ` : ''}${a.gas_tgas ? `${a.gas_tgas} TGas` : ''}</div>
        <details><summary class="small">Exact arguments</summary><pre class="mono small">${esc(JSON.stringify(a.args, null, 2))}</pre></details></li>`,
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
    try {
      status('Waiting for your passkey…');
      const out = await signAndRelay(msg, ctx.pendingId);
      if (!out.ok) throw new Error('Refused on-chain: ' + JSON.stringify(out.failures));
      const done = ctx.after ? await ctx.after(out) : `Done. Transaction ${esc(out.tx_hash)}`;
      m.innerHTML = '';
      await dashboard();
      const el = document.getElementById('status');
      if (el) {
        el.className = 'status ok';
        el.innerHTML = done;
      }
    } catch (e) {
      status(String((e as Error).message ?? e), 'error');
    }
  };
}

main().catch((e) => render(`<section class="card"><h1>Marvii</h1><p class="error">${esc(e.message ?? e)}</p><p class="muted small">Is the relayer running at the configured address?</p></section>`));
