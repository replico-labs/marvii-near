// Owner send on Base Sepolia from an existing account, through the page:
// Send → Confirm → passkey → broadcast. The account's passkey key is loaded
// into Chromium's virtual authenticator from DEMO_PASSKEY (a testnet key
// file kept outside the repo; never printed).
//   DEMO_PASSKEY=…/demo-passkey.json WALLET=0s… [CHAIN=84532|solana|bitcoin] node e2e/send.mjs <amount> <to>
// On EVM chains it sends USDC; on Solana and Bitcoin the native coin.
import { createRequire } from 'node:module';
import crypto from 'node:crypto';
import fs from 'node:fs';
const { chromium } = createRequire(import.meta.url)('playwright');

const [amount = '1', to = '0x000000000000000000000000000000000000dEaD'] = process.argv.slice(2);
const seed = Buffer.from(JSON.parse(fs.readFileSync(process.env.DEMO_PASSKEY, 'utf8')).seed_hex, 'hex');
const ecdh = crypto.createECDH('prime256v1');
ecdh.setPrivateKey(seed);
const pub = ecdh.getPublicKey(); // 04 || x || y
const b64u = (b) => Buffer.from(b).toString('base64url');
const key = crypto.createPrivateKey({ key: { kty: 'EC', crv: 'P-256', d: b64u(seed), x: b64u(pub.subarray(1, 33)), y: b64u(pub.subarray(33)) }, format: 'jwk' });
const pkcs8 = key.export({ format: 'der', type: 'pkcs8' }).toString('base64');
const credentialId = crypto.randomBytes(16);

const browser = await chromium.launch(process.env.HTTPS_PROXY ? { args: [`--proxy-server=${process.env.HTTPS_PROXY}`, '--proxy-bypass-list=localhost;127.0.0.1'] } : {});
const page = await browser.newPage();
const cdp = await page.context().newCDPSession(page);
await cdp.send('WebAuthn.enable');
const { authenticatorId } = await cdp.send('WebAuthn.addVirtualAuthenticator', {
  options: { protocol: 'ctap2', transport: 'internal', hasResidentKey: true, hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true },
});
await cdp.send('WebAuthn.addCredential', { authenticatorId, credential: { credentialId: credentialId.toString('base64'), isResidentCredential: true, rpId: 'localhost', privateKey: pkcs8, userHandle: crypto.randomBytes(8).toString('base64'), signCount: 1 } });
page.on('pageerror', (e) => console.log('page error:', e.message));
await page.goto('http://localhost:5173');
await page.evaluate(([id, pk, w]) => localStorage.setItem('marvii-near:passkey', JSON.stringify({ credentialId: id, publicKey: pk, wallet: w })), [b64u(credentialId), pub.toString('hex'), process.env.WALLET]);
await page.reload();
const chain = process.env.CHAIN ?? '84532';
await page.waitForSelector(`#chain-${chain} [data-send]`, { state: 'attached', timeout: 90_000 });
await page.evaluate((c) => (location.hash = `#/chain-${c}`), chain);
// The dashboard loads every chain before the page is usable.
await page.waitForSelector(`#chain-${chain} [data-send]`, { state: 'visible', timeout: 90_000 });
await page.waitForSelector('#busy', { state: 'detached', timeout: 90_000 });
await page.click(`#chain-${chain} [data-send]`);
if (await page.$('#send select[name=asset]')) {
  const opts = await page.$$eval('#send select[name=asset] option', (o) => o.map((x) => [x.value, x.textContent]));
  await page.selectOption('#send select[name=asset]', opts.find(([, t]) => t.startsWith('USDC'))[0]);
}
await page.fill('#send input[name=to]', to);
await page.fill('#send input[name=amount]', amount);
await page.click('#send button.primary');
await page.waitForSelector('.sheet >> text=Approve:', { timeout: 60_000 }).catch(async (e) => {
  const modal = await page.textContent('#modal').catch(() => '?');
  console.log('send form says:', await page.textContent('#send #status').catch(() => '?'), '| modal:', modal.replace(/\s+/g, ' ').slice(0, 400));
  throw e;
});
console.log('Confirm shows:', (await page.textContent('.sheet .list')).replace(/\s+/g, ' ').trim().slice(0, 200));
await page.click('#yes');
await page.waitForSelector('.status.ok, .sheet .status.error', { timeout: 180_000 });
const err = await page.$('.sheet .status.error');
if (err) console.log('error:', await err.textContent());
const ok = await page.$('.status.ok a');
console.log(ok ? `sent: ${await ok.getAttribute('href')}` : 'no transaction link');
await browser.close();
process.exit(ok ? 0 : 1);
