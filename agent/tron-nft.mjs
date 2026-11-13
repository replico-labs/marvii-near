// Test collection for the TRON demo (Nile): deploys contracts/MarviiDemoNFT.sol
// from a local key kept in MARVII_NEAR_KEYS (tron-nft-deployer.json, never
// printed), and mints NFTs to an address. TronWeb only signs, locally; the
// node is called with fetch (which honours the proxy).
//
//   node tron-nft.mjs address                the deployer's address (fund it first)
//   node tron-nft.mjs deploy <combined.json> compile output of solc --combined-json abi,bin
//   node tron-nft.mjs mint <contract> <to>
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { TronWeb } from 'tronweb';

const NILE = process.env.TRON_API ?? 'https://nile.trongrid.io';
const KEYS = process.env.MARVII_NEAR_KEYS;
const f = path.join(KEYS, 'tron-nft-deployer.json');
const offline = new TronWeb({ fullHost: NILE });
if (!fs.existsSync(f)) {
  const pk = crypto.randomBytes(32).toString('hex');
  fs.writeFileSync(f, JSON.stringify({ private_key: pk, address: offline.address.fromPrivateKey(pk) }));
}
const key = JSON.parse(fs.readFileSync(f, 'utf8'));
const tw = new TronWeb({ fullHost: NILE, privateKey: key.private_key });
const api = async (p, body) => (await fetch(NILE + p, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) })).json();

async function signSend(tx) {
  const signed = await tw.trx.sign(tx, key.private_key);
  const r = await api('/wallet/broadcasttransaction', signed);
  if (!r.result) throw new Error(JSON.stringify(r));
  for (let i = 0; i < 30; i++) {
    await new Promise((r) => setTimeout(r, 3000));
    const info = await api('/wallet/gettransactioninfobyid', { value: tx.txID });
    if (info.id) return info;
  }
  throw new Error('not confirmed');
}

const [cmd, a1, a2] = process.argv.slice(2);
if (cmd === 'address') {
  const a = await api('/wallet/getaccount', { address: key.address, visible: true });
  console.log(`${key.address}: ${(a.balance ?? 0) / 1e6} TRX`);
} else if (cmd === 'deploy') {
  const c = Object.entries(JSON.parse(fs.readFileSync(a1, 'utf8')).contracts).find(([k]) => k.endsWith(':MarviiDemoNFT'))[1];
  const abi = typeof c.abi === 'string' ? JSON.parse(c.abi) : c.abi;
  const tx = await api('/wallet/deploycontract', { owner_address: key.address, abi: JSON.stringify(abi), bytecode: c.bin, fee_limit: 1_000_000_000, origin_energy_limit: 10_000_000, consume_user_resource_percent: 100, name: 'MarviiDemoNFT', visible: true });
  if (!tx.txID) throw new Error(JSON.stringify(tx));
  const info = await signSend(tx);
  console.log(`deployed ${offline.address.fromHex(info.contract_address)} (tx ${tx.txID}, ${info.receipt?.result})`);
} else if (cmd === 'mint') {
  const to = offline.address.toHex(a2).slice(2).padStart(64, '0');
  const r = await api('/wallet/triggersmartcontract', { owner_address: key.address, contract_address: a1, function_selector: 'mint(address)', parameter: to, fee_limit: 100_000_000, visible: true });
  const info = await signSend(r.transaction);
  const id = info.log?.[0]?.topics?.[3];
  console.log(`minted token ${id ? BigInt('0x' + id) : '?'} to ${a2} (tx ${r.transaction.txID}, ${info.receipt?.result})`);
}
