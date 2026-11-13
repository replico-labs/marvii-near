// The same flow with the session key inside OutLayer's TEE: the agent never
// sees its key. Each signature is an OutLayer run, paid by the operator
// account (NEAR testnet), and the module signs only extension messages it
// builds itself.
//
// Env: OUTLAYER_OPERATOR (account id), OUTLAYER_OPERATOR_KEY (ed25519:… of
// that account; from the environment, never printed), MODULE_URL and
// MODULE_HASH (the published module), MARVII_PAGE, MARVII_RELAYER.
import { OutLayerSigner, permissionLink, waitForApproval, MarviiAgent, nearCall } from '../index.mjs';

const page = process.env.MARVII_PAGE, relayer = process.env.MARVII_RELAYER;
const signer = new OutLayerSigner({
  moduleUrl: process.env.MODULE_URL,
  moduleHash: process.env.MODULE_HASH,
  operator: { accountId: process.env.OUTLAYER_OPERATOR, privateKey: process.env.OUTLAYER_OPERATOR_KEY },
});
const publicKey = await signer.getPublicKey();
console.log(`TEE key ${publicKey}`);
console.log(`Open this link and approve with your passkey:\n${permissionLink({
  publicKey, keyId: 'tee-quickstart', app: { name: 'TEE agent (OutLayer)' }, days: 1,
  grant: { near_venues: [{ receiver_id: 'wrap.testnet', method: 'near_deposit' }], caps: [{ asset: 'near', daily: '50000000000000000000000' }] },
  ...(page && { page }),
})}\n`);
const { wallet } = await waitForApproval({ publicKey, ...(relayer && { relayer }) });
console.log(`approved by ${wallet}`);
const agent = new MarviiAgent({ wallet, keyId: 'tee-quickstart', signer, ...(relayer && { relayer }) });
const r = await agent.execute(nearCall('wrap.testnet', 'near_deposit', {}, '10000000000000000000000'));
console.log('wrap 0.01 NEAR, signed in the TEE:', r.ok ? `done, NEAR tx ${r.tx_hash}` : `refused: ${r.reason ?? r.error}`);
