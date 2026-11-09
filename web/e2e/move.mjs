// Moving an account to a newer Marvii extension, in Chromium with a virtual
// passkey: the account is created while the page is told the previous
// extension is current (as if created before the update), then the real
// configuration loads, the page offers the update, and one passkey tap adds
// the current extension and removes the old one. The address stays.
//
// PAGE and RELAYER default to localhost:5173 and :8787. OLD is the previous
// extension (default: v3, 2dc4e98d…).
import { createRequire } from 'node:module';
const { chromium } = createRequire(import.meta.url)('playwright');

const PAGE = process.env.PAGE ?? 'http://localhost:5173';
const RELAYER = process.env.RELAYER ?? 'http://localhost:8787';
const OLD = process.env.OLD ?? '2dc4e98d9de62211b1263a9a75d3c71caf45795719b790f12921198af309ba18';
const cfg = await (await fetch(RELAYER + '/config')).json();
if (!cfg.known_extensions?.includes(OLD)) throw new Error('the relayer does not know the old extension');

async function extensions(wallet) {
  const args = Buffer.from('{}').toString('base64');
  const r = await fetch(cfg.rpc, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'query', params: { request_type: 'call_function', finality: 'final', account_id: wallet, method_name: 'w_extensions', args_base64: args } }) }).then((r) => r.json());
  return JSON.parse(Buffer.from(r.result.result).toString());
}

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
await page.route(`${RELAYER}/config`, (route) => route.fulfill({ json: { ...cfg, extension: OLD } }));
await page.goto(PAGE);
await page.click('#create');
await page.waitForSelector('.tiles3', { timeout: 240_000 });
const wallet = await page.evaluate(() => JSON.parse(localStorage.getItem('marvii-near:passkey')).wallet);
console.log('created', wallet, 'with', (await extensions(wallet)).map((e) => e.slice(0, 8)));

await page.unroute(`${RELAYER}/config`);
await page.reload();
await page.waitForSelector('#move-card:not([hidden])', { timeout: 90_000 });
console.log('offered:', (await page.textContent('#move-card p')).trim().slice(0, 80) + '…');
await page.click('#move');
await page.waitForSelector('.sheet >> text=Move to the current Marvii extension', { timeout: 30_000 });
console.log('confirm shows:', (await page.textContent('.sheet .list')).replace(/\s+/g, ' ').trim());
const blocked = await page.$eval('#yes', (b) => b.disabled);
if (blocked) throw new Error('Confirm blocked the move');
await page.click('#yes');
let after = [];
for (let i = 0; i < 30; i++) {
  await page.waitForTimeout(3000);
  after = await extensions(wallet);
  if (after.includes(cfg.extension)) break;
}
console.log('now', after.map((e) => e.slice(0, 8)));
const ok = after.includes(cfg.extension) && !after.includes(OLD);
await page.reload();
await page.waitForSelector('.tiles3', { timeout: 60_000 });
await page.waitForTimeout(5000);
const still = await page.$eval('#move-card', (e) => !e.hidden);
console.log('update card after the move:', still ? 'still shown' : 'gone');
await browser.close();
console.log(ok && !still ? 'PASS' : 'FAIL');
process.exit(ok && !still ? 0 : 1);
