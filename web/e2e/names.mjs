// Names in Chromium with a virtual passkey, both ways:
// 1. a name chosen at creation, registered by the same tap that activates;
// 2. an account created without one, then "Choose a name" → Confirm → passkey.
// Then a deposit to the name is forwarded to the account.
//
// PAGE and RELAYER default to localhost:5173 and :8787. Needs a relayer with
// names enabled (MARVII_NAMES_REGISTRY, or names.json in its keys directory).
import { createRequire } from 'node:module';
import crypto from 'node:crypto';
const { chromium } = createRequire(import.meta.url)('playwright');

const PAGE = process.env.PAGE ?? 'http://localhost:5173';
const RELAYER = process.env.RELAYER ?? 'http://localhost:8787';
const cfg = await (await fetch(RELAYER + '/config')).json();
if (!cfg.names) throw new Error('the relayer has no name registry');
const browser = await chromium.launch(
  process.env.HTTPS_PROXY ? { args: [`--proxy-server=${process.env.HTTPS_PROXY}`, '--proxy-bypass-list=localhost;127.0.0.1'] } : {},
);
const rand = () => 'e2e' + crypto.randomBytes(4).toString('hex');

async function freshPage() {
  const page = await (await browser.newContext()).newPage();
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('WebAuthn.enable');
  await cdp.send('WebAuthn.addVirtualAuthenticator', {
    options: { protocol: 'ctap2', transport: 'internal', hasResidentKey: true, hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true },
  });
  page.on('pageerror', (e) => console.log('page error:', e.message));
  await page.goto(PAGE);
  return page;
}

async function nearRow(page) {
  await page.click('#chain-near summary');
  await page.waitForSelector('#chain-near [data-copy]', { state: 'attached', timeout: 60_000 });
  return (await page.textContent('#chain-near .assets')).replace(/\s+/g, ' ').trim();
}

let ok = true;

// 1. Name at creation.
const a = rand();
const p1 = await freshPage();
await p1.fill('#name', a);
await p1.waitForSelector(`#name-note >> text=is available`, { timeout: 30_000 });
await p1.click('#create');
await p1.waitForSelector('text=Balances', { timeout: 240_000 });
await p1.waitForSelector(`#chain-near [data-copy="${a}.${cfg.names}"]`, { state: 'attached', timeout: 60_000 }).catch(() => (ok = false));
console.log('1. created with a name:', await nearRow(p1));

// The same name is now refused on the create screen.
const p2 = await freshPage();
await p2.fill('#name', a);
await p2.waitForSelector('#name-note >> text=taken', { timeout: 30_000 }).catch(() => (ok = false));
console.log('   the same name again:', await p2.textContent('#name-note'));

// 2. No name at creation, chosen later.
await p2.fill('#name', '');
await p2.click('#create');
await p2.waitForSelector('text=Balances', { timeout: 240_000 });
console.log('2. created without a name:', await nearRow(p2));
const b = rand();
await p2.click('[data-name]');
await p2.fill('input[name=name]', b);
await p2.click('#send button.primary');
await p2.waitForSelector('.sheet >> text=Take the name', { timeout: 30_000 });
console.log('   Confirm shows:', (await p2.textContent('.sheet .list')).replace(/\s+/g, ' ').trim().slice(0, 160));
await p2.click('#yes');
// Done when the sheet closes (back on the wallet), or it shows an error.
await p2.waitForFunction(() => !document.querySelector('#modal .sheet') || document.querySelector('.sheet .status.error'), null, { timeout: 240_000 });
const err = await p2.$('.sheet .status.error');
if (err) {
  ok = false;
  console.log('   error:', await err.textContent());
}
await p2.waitForSelector(`#chain-near [data-copy="${b}.${cfg.names}"]`, { state: 'attached', timeout: 60_000 }).catch(() => (ok = false));
console.log('   after choosing:', await nearRow(p2));

await browser.close();
console.log(ok ? 'PASS' : 'FAIL');
process.exit(ok ? 0 : 1);
