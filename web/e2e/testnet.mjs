// End-to-end on NEAR testnet in Chromium with a virtual passkey:
// create → activate → grant an agent → agent's EVM request signed through
// Chain Signatures → revoke → agent refused.
//
// Needs the relayer on :8787 and `npm run dev` on :5173. The EVM contract
// addresses below are placeholders for this test only: the transaction is
// signed, never broadcast.
import { createRequire } from 'node:module';
import crypto from 'node:crypto';
const { chromium } = createRequire(import.meta.url)('playwright');

const PAGE = 'http://localhost:5173';
const RELAYER = 'http://localhost:8787';
const POOL = '0x2222222222222222222222222222222222222222';
const USDC = '0x4444444444444444444444444444444444444444';

const cfg = await (await fetch(RELAYER + '/config')).json();

// The agent's key lives here, never in the page.
const { publicKey, privateKey } = crypto.generateKeyPairSync('ed25519');
const agentPk = publicKey.export({ format: 'der', type: 'spki' }).subarray(-32).toString('hex');

// In a sandbox with an egress proxy, set HTTPS_PROXY; localhost stays direct.
// (Playwright's own proxy option routes loopback through the proxy too.)
const browser = await chromium.launch(
  process.env.HTTPS_PROXY ? { args: [`--proxy-server=${process.env.HTTPS_PROXY}`, '--proxy-bypass-list=localhost;127.0.0.1'] } : {},
);
const page = await browser.newPage();
const cdp = await page.context().newCDPSession(page);
await cdp.send('WebAuthn.enable');
await cdp.send('WebAuthn.addVirtualAuthenticator', {
  options: { protocol: 'ctap2', transport: 'internal', hasResidentKey: true, hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true },
});
page.on('pageerror', (e) => console.log('page error:', e.message));

await page.goto(PAGE);
await page.click('#create');
await page.waitForSelector('text=Balances', { timeout: 120_000 });
await page.waitForSelector('#chain-near [data-copy]', { state: 'attached', timeout: 60_000 });
const wallet = await page.getAttribute('#chain-near [data-copy]', 'data-copy');
console.log('NEAR account', wallet, 'balance', await page.textContent('#chain-near .bal'));
await page.waitForSelector('#chain-84532 [data-copy]', { state: 'attached', timeout: 60_000 });
console.log('EVM address (every EVM chain)', await page.getAttribute('#chain-84532 [data-copy]', 'data-copy'));

// Grant: approve to the pool only, up to 50 USDC a day.
await page.click('#add');
await page.fill('input[name=public_key]', agentPk);
await page.fill('input[name=to]', POOL);
await page.fill('input[name=selector]', '0x617ba037');
await page.fill('input[name=token]', USDC);
await page.check('input[name=queue]');
await page.click('#grant button.primary');
await page.waitForSelector('text=Confirm');
console.log('confirm shows:', (await page.textContent('.sheet .list')).replace(/\s+/g, ' ').trim().slice(0, 300));
await page.click('#yes');
await page.waitForSelector('.pill.ok', { timeout: 120_000 });
console.log('session active on the dashboard');

async function agent(n, amount, path = '/agent/execute', data = null) {
  data ??= '0x095ea7b3' + POOL.slice(2).padStart(64, '0') + amount.toString(16).padStart(64, '0');
  const tx = { chain_id: 84532, nonce: n - 1, max_priority_fee_per_gas: '1000000', max_fee_per_gas: '2000000000', gas_limit: 100000, to: USDC, value: '0', data };
  const msg = JSON.stringify({ domain: 'marvii-session-v1', extension: cfg.extension, wallet, key_id: 'agent-1', nonce: n, deadline_ms: Date.now() + 300_000, action: { type: 'evm_tx', tx } });
  const digest = crypto.createHash('sha256').update(msg).digest();
  const signature = crypto.sign(null, digest, privateKey).toString('hex');
  const r = await fetch(RELAYER + path, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ msg, signature }) });
  const j = await r.json();
  j.reason = j.reason ?? j.error ?? '';
  return j;
}

const ok = await agent(1, 20_000_000);
console.log('agent approve 20 USDC:', ok.ok, 'tx', ok.tx_hash, 'mpc signature', ok.mpc_signature ? 'returned' : 'missing', 'hash', ok.evm_signing_hash);
const over = await agent(2, 40_000_000);
console.log('agent approve 40 more (over 50/day):', over.ok ? 'ACCEPTED (bad)' : 'refused:', over.reason);

// Over the cap: the request waits for the owner. Approve it with the passkey.
let approved = null;
if (over.pending_id) {
  await page.reload();
  await page.waitForSelector(`[data-approve="${over.pending_id}"]`, { timeout: 60_000 });
  console.log('pending shows:', (await page.textContent('#pending li div')).replace(/\s+/g, ' ').trim());
  await page.click(`[data-approve="${over.pending_id}"]`);
  await page.waitForSelector('.sheet >> text=Approve:');
  await page.click('#yes');
  await page.waitForSelector('.status.ok, .sheet .status.error', { timeout: 180_000 });
  const err = await page.$('.sheet .status.error');
  if (err) console.log('approval error:', await err.textContent());
  approved = await (await fetch(`${RELAYER}/pending/${over.pending_id}`)).json();
  console.log('owner approval:', approved.status, approved.mpc_signature ? 'MPC signature for the agent' : 'no signature', 'NEAR tx', approved.near_tx);
}

// A new recipient: refused directly, then proposed into the delay queue,
// where the passkey cancels it.
const STRANGER = '0x6666666666666666666666666666666666666666';
const t = '0xa9059cbb' + STRANGER.slice(2).padStart(64, '0') + (5_000_000).toString(16).padStart(64, '0');
const direct = await agent(2, 0, '/agent/execute', t);
console.log('agent pays a new recipient directly:', direct.ok ? 'ACCEPTED (bad)' : 'refused:', direct.reason);
const prop = await agent(2, 0, '/agent/propose', t); // the refusal above didn't use nonce 2
console.log('agent proposes it instead: queued as', prop.queue_id, 'ready', new Date(prop.ready_at_ms).toISOString());
await page.reload();
await page.waitForSelector(`[data-cancel="${prop.queue_id}"]`, { timeout: 60_000 });
console.log('queued shows:', (await page.textContent('#queued li')).replace(/\s+/g, ' ').trim().slice(0, 160));
await page.click(`[data-cancel="${prop.queue_id}"]`);
await page.waitForSelector('text=Cancel queued request');
await page.click('#yes');
await page.waitForSelector('.status.ok, .sheet .status.error', { timeout: 180_000 });
const left = await (await fetch(cfg.rpc, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'query', params: { request_type: 'call_function', finality: 'final', account_id: cfg.extension, method_name: 'list_queued', args_base64: Buffer.from(JSON.stringify({ wallet })).toString('base64') } }) })).json();
const queuedLeft = JSON.parse(Buffer.from(left.result.result).toString()).length;
console.log('after the passkey cancels, queued requests left:', queuedLeft);

await page.click('[data-revoke]');
await page.waitForSelector('text=Revoke the session key');
await page.click('#yes');
await page.waitForSelector('.pill.bad', { timeout: 120_000 });
console.log('revoked on the dashboard');
const after = await agent(3, 1_000_000);
console.log('agent after revoke:', after.ok ? 'ACCEPTED (bad)' : 'refused:', after.reason);

await page.screenshot({ path: process.env.SHOT ?? 'dashboard.png', fullPage: true });
await browser.close();
const pass = ok.ok && ok.mpc_signature && !over.ok && over.reason.includes('daily cap') && approved?.status === 'approved' && approved.mpc_signature && !direct.ok && direct.reason === 'recipient not allowed' && prop.queue_id && queuedLeft === 0 && !after.ok && after.reason.includes('revoked');
console.log(pass ? 'PASS' : 'FAIL');
process.exit(pass ? 0 : 1);
