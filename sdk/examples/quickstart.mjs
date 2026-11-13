// Quickstart: an agent asks for permission, waits for the owner's passkey,
// then acts within it. Here it may wrap up to 0.05 NEAR a day into wNEAR
// on NEAR testnet, and nothing else.
//
//   node examples/quickstart.mjs            prints the link, waits, acts
// Env: MARVII_PAGE, MARVII_RELAYER (defaults: near.marvii.xyz, relayer.marvii.xyz);
//      AGENT_KEY_FILE to keep the session key between runs.
import fs from 'node:fs';
import { generateKey, LocalSigner, permissionLink, waitForApproval, MarviiAgent, nearCall } from '../index.mjs';

const page = process.env.MARVII_PAGE, relayer = process.env.MARVII_RELAYER;
const file = process.env.AGENT_KEY_FILE ?? 'agent-key.json';
const key = fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, 'utf8')) : generateKey();
fs.writeFileSync(file, JSON.stringify(key), { mode: 0o600 });
const signer = new LocalSigner(key.seed_hex);

const link = permissionLink({
  publicKey: signer.publicKey,
  keyId: 'quickstart',
  app: { name: 'Quickstart agent', url: 'https://github.com/replico-labs/marvii-near' },
  days: 1,
  grant: {
    near_venues: [{ receiver_id: 'wrap.testnet', method: 'near_deposit' }],
    caps: [{ asset: 'near', daily: '50000000000000000000000' }], // 0.05 NEAR
  },
  ...(page && { page }),
});
console.log(`Open this link and approve with your passkey:\n${link}\n`);

const { wallet } = await waitForApproval({ publicKey: signer.publicKey, ...(relayer && { relayer }) });
console.log(`approved by ${wallet}`);
const agent = new MarviiAgent({ wallet, keyId: 'quickstart', signer, ...(relayer && { relayer }) });

const ok = await agent.execute(nearCall('wrap.testnet', 'near_deposit', {}, '10000000000000000000000'));
console.log('wrap 0.01 NEAR:', ok.ok ? `done, NEAR tx ${ok.tx_hash}` : `refused: ${ok.reason ?? ok.error}`);
const over = await agent.execute(nearCall('wrap.testnet', 'near_deposit', {}, '50000000000000000000000'));
console.log('wrap 0.05 more (over the cap):', over.ok ? 'done?!' : `refused: ${over.reason ?? over.error}`);
const other = await agent.execute(nearCall('meta-v2.pool.testnet', 'deposit_and_stake', {}, '10000000000000000000000'));
console.log('stake with Meta Pool (not granted):', other.ok ? 'done?!' : `refused: ${other.reason ?? other.error}`);
