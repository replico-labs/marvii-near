// A permission link from near/sdk, opened as the owner: the page shows the
// app's request on the Confirm screen and the passkey approves it.
//   DEMO_PASSKEY=…/demo-passkey.json WALLET=0s… node e2e/connect.mjs '<link>'
// The link must point at the local page (http://localhost:5173).
import { createRequire } from 'node:module';
import crypto from 'node:crypto';
import fs from 'node:fs';
const { chromium } = createRequire(import.meta.url)('playwright');

const link = process.argv[2];
if (!link) throw new Error('usage: connect.mjs <permission link>');
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
// The owner opens the link: the page fills in the account's addresses and
// shows the grant on the Confirm screen; the passkey approves it.
await page.goto(link);
await page.waitForSelector('.sheet >> text=Give', { timeout: 120_000 }).catch(async (e) => {
  console.log('page says:', (await page.textContent('#status').catch(() => '?'))?.replace(/\s+/g, ' '));
  throw e;
});
console.log('Confirm shows:', (await page.textContent('.sheet')).replace(/\s+/g, ' ').trim().slice(0, 600));
if (await page.$eval('#yes', (b) => b.disabled)) throw new Error('approval is blocked');
await page.click('#yes');
await page.waitForSelector('.sheet .status.ok, .status.ok, .sheet .status.error', { timeout: 180_000 });
const err = await page.$('.sheet .status.error');
console.log(err ? `error: ${await err.textContent()}` : `page: ${(await page.textContent('.status.ok')).replace(/\s+/g, ' ').trim()}`);
await browser.close();
