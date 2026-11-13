// Demo agent on Ethereum testnets: Lido staking on Hoodi and Seaport (the
// contract OpenSea trades through) on Base Sepolia. The agent holds only its
// session key; the extension checks each transaction or order and the MPC
// network signs it with the account's EVM key (path `evm`).
//
//   node eth.mjs address                         the account and its balances
//   node eth.mjs lido-stake <eth>                stake ETH for stETH (Hoodi)
//   node eth.mjs lido-wrap <steth>               wrap stETH into wstETH
//   node eth.mjs lido-unwrap <wsteth>            and back
//   node eth.mjs lido-withdraw <steth>           request a withdrawal (the request NFT is the account's)
//   node eth.mjs lido-withdraw-to <steth> <0x>   the same for someone else (refused)
//   node eth.mjs lido-claim <request id>         claim a finalised withdrawal
//   node eth.mjs seaport-approve                 let Seaport move the test collection's NFTs
//   node eth.mjs seaport-list <id> <eth> [file]  sign a listing; writes the order to [file]
//   node eth.mjs seaport-buy <order file>        buy a listed NFT (a signed order, as Seaport takes it)
//   node eth.mjs seaport-cancel <order file>     cancel the account's listing
//
// Env as agent.mjs: MARVII_NEAR_KEYS, RELAYER, WALLET.
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import {
  createPublicClient, http, keccak256, serializeTransaction, recoverTransactionAddress, recoverAddress,
  encodeFunctionData, parseAbi, parseEther, formatEther, hashTypedData, serializeSignature,
} from 'viem';
import { baseSepolia, hoodi } from 'viem/chains';

const RELAYER = process.env.RELAYER ?? 'http://localhost:8787';
const KEYS = process.env.MARVII_NEAR_KEYS;
const WALLET = process.env.WALLET;
const KEY_ID = process.env.KEY_ID ?? 'aave-agent';
const N = 0xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141n;

// Lido on Hoodi (docs.lido.fi/deployed-contracts/hoodi), checked on-chain:
// the Lido Locator's lido() and withdrawalQueue() are these, and wstETH's
// and the queue's stETH() are the stETH below.
const LIDO = {
  steth: '0x3508A952176b3c15387C97BE809eaffB1982176a',
  wsteth: '0x7E99eE3C66636DE415D2d7C880938F2f40f94De4',
  queue: '0xfe56573178f1bcdf53F01A6E9977670dcBBD9186',
};
// Seaport 1.6 on Base Sepolia (information() reports "1.6"), and a test
// collection deployed for this demo (anyone may mint).
const SEAPORT = '0x0000000000000068F116a894984e2DB1123eB395';
const TEST_NFT = '0xFF67823cCb8D504D0627609F4D1dCA86b2B51C43';

const CHAINS = {
  [hoodi.id]: { chain: hoodi, rpc: process.env.HOODI_RPC ?? 'https://ethereum-hoodi-rpc.publicnode.com', explorer: 'https://hoodi.etherscan.io/tx/' },
  [baseSepolia.id]: { chain: baseSepolia, rpc: process.env.EVM_RPC ?? 'https://sepolia.base.org', explorer: 'https://sepolia.basescan.org/tx/' },
};
const client = (id) => createPublicClient({ chain: CHAINS[id].chain, transport: http(CHAINS[id].rpc) });

const abi = parseAbi([
  'function submit(address referral) payable returns (uint256)',
  'function wrap(uint256) returns (uint256)',
  'function unwrap(uint256) returns (uint256)',
  'function approve(address spender, uint256 amount) returns (bool)',
  'function balanceOf(address) view returns (uint256)',
  'function requestWithdrawals(uint256[] amounts, address owner) returns (uint256[])',
  'function claimWithdrawal(uint256 requestId)',
  'function getWithdrawalRequests(address owner) view returns (uint256[])',
  'function setApprovalForAll(address operator, bool approved)',
  'function getCounter(address offerer) view returns (uint256)',
  'function ownerOf(uint256) view returns (address)',
  'function fulfillBasicOrder((address considerationToken, uint256 considerationIdentifier, uint256 considerationAmount, address offerer, address zone, address offerToken, uint256 offerIdentifier, uint256 offerAmount, uint8 basicOrderType, uint256 startTime, uint256 endTime, bytes32 zoneHash, uint256 salt, bytes32 offererConduitKey, bytes32 fulfillerConduitKey, uint256 totalOriginalAdditionalRecipients, (uint256 amount, address recipient)[] additionalRecipients, bytes signature) parameters) payable returns (bool)',
  'function cancel((address offerer, address zone, (uint8 itemType, address token, uint256 identifierOrCriteria, uint256 startAmount, uint256 endAmount)[] offer, (uint8 itemType, address token, uint256 identifierOrCriteria, uint256 startAmount, uint256 endAmount, address recipient)[] consideration, uint8 orderType, uint256 startTime, uint256 endTime, bytes32 zoneHash, uint256 salt, bytes32 conduitKey, uint256 counter)[] orders) returns (bool)',
]);

// Seaport 1.6's EIP-712 types, as seaport-js signs them.
const ORDER_TYPES = {
  OrderComponents: [
    { name: 'offerer', type: 'address' }, { name: 'zone', type: 'address' }, { name: 'offer', type: 'OfferItem[]' },
    { name: 'consideration', type: 'ConsiderationItem[]' }, { name: 'orderType', type: 'uint8' }, { name: 'startTime', type: 'uint256' },
    { name: 'endTime', type: 'uint256' }, { name: 'zoneHash', type: 'bytes32' }, { name: 'salt', type: 'uint256' },
    { name: 'conduitKey', type: 'bytes32' }, { name: 'counter', type: 'uint256' },
  ],
  OfferItem: [
    { name: 'itemType', type: 'uint8' }, { name: 'token', type: 'address' }, { name: 'identifierOrCriteria', type: 'uint256' },
    { name: 'startAmount', type: 'uint256' }, { name: 'endAmount', type: 'uint256' },
  ],
  ConsiderationItem: [
    { name: 'itemType', type: 'uint8' }, { name: 'token', type: 'address' }, { name: 'identifierOrCriteria', type: 'uint256' },
    { name: 'startAmount', type: 'uint256' }, { name: 'endAmount', type: 'uint256' }, { name: 'recipient', type: 'address' },
  ],
};
const Z = '0x0000000000000000000000000000000000000000';
const Z32 = `0x${'00'.repeat(32)}`;

function agentKey() {
  const { seed_hex } = JSON.parse(fs.readFileSync(path.join(KEYS, 'demo-agent.json'), 'utf8'));
  return crypto.createPrivateKey({ key: Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), Buffer.from(seed_hex, 'hex')]), format: 'der', type: 'pkcs8' });
}
async function relayer(p, body) {
  return (await fetch(RELAYER + p, body ? { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) } : {})).json();
}
async function view(rpc, account_id, method_name, args) {
  const r = await fetch(rpc, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'query', params: { request_type: 'call_function', finality: 'final', account_id, method_name, args_base64: Buffer.from(JSON.stringify(args)).toString('base64') } }) }).then((r) => r.json());
  if (r.error || r.result?.error) throw new Error(JSON.stringify(r.error ?? r.result.error));
  return JSON.parse(Buffer.from(r.result.result).toString());
}

const cfg = await relayer('/config');
const session = await view(cfg.rpc, cfg.extension, 'get_session', { wallet: WALLET, key_id: KEY_ID });
if (!session) throw new Error(`no session ${KEY_ID} for this wallet`);
const ME = session.grant.evm_self;

/** Asks the extension to sign the action; returns the MPC signature or null. */
async function execute(action) {
  const s = await view(cfg.rpc, cfg.extension, 'get_session', { wallet: WALLET, key_id: KEY_ID });
  const msg = JSON.stringify({ domain: 'marvii-session-v1', extension: cfg.extension, wallet: WALLET, key_id: KEY_ID, nonce: s.nonce + 1, deadline_ms: Date.now() + 300_000, action });
  const out = await relayer('/agent/execute', { msg, signature: crypto.sign(null, crypto.createHash('sha256').update(msg).digest(), agentKey()).toString('hex') });
  console.log(`  NEAR tx ${out.tx_hash ?? '-'}`);
  if (!out.ok || !out.mpc_signature) {
    console.log(`  REFUSED by the extension: ${out.reason ?? out.error ?? 'no signature'}`);
    return null;
  }
  return out.mpc_signature;
}

/** The MPC signature as Ethereum's r, s (low s) and y parity. */
function rsv(sig) {
  const r = BigInt('0x' + sig.big_r.affine_point.slice(2));
  let s = BigInt('0x' + sig.s.scalar);
  let yParity = sig.recovery_id;
  if (s > N / 2n) { s = N - s; yParity ^= 1; }
  return { r: '0x' + r.toString(16).padStart(64, '0'), s: '0x' + s.toString(16).padStart(64, '0'), yParity };
}

/** Sends a transaction through the extension; returns whether it succeeded. */
async function send(chainId, to, data, value = 0n) {
  const c = client(chainId);
  const [nonce, block, prio, gas] = await Promise.all([
    c.getTransactionCount({ address: ME, blockTag: 'pending' }),
    c.getBlock(),
    c.estimateMaxPriorityFeePerGas(),
    c.estimateGas({ account: ME, to, data, value }).catch((e) => { console.log(`  (gas estimate failed: ${e.shortMessage ?? e.message}; using 300000)`); return 250_000n; }),
  ]);
  const unsigned = { type: 'eip1559', chainId, nonce, to, value, data, maxPriorityFeePerGas: prio, maxFeePerGas: block.baseFeePerGas * 2n + prio, gas: (gas * 12n) / 10n };
  const sig = await execute({ type: 'evm_tx', tx: {
    chain_id: chainId, nonce, max_priority_fee_per_gas: prio.toString(), max_fee_per_gas: unsigned.maxFeePerGas.toString(),
    gas_limit: Number(unsigned.gas), to, value: value.toString(), data,
  } });
  if (!sig) return false;
  const signed = serializeTransaction(unsigned, rsv(sig));
  const sender = await recoverTransactionAddress({ serializedTransaction: signed });
  if (sender.toLowerCase() !== ME.toLowerCase()) throw new Error(`signature recovers to ${sender}, not ${ME}`);
  const hash = await c.sendRawTransaction({ serializedTransaction: signed });
  const rc = await c.waitForTransactionReceipt({ hash });
  console.log(`  ${CHAINS[chainId].chain.name} tx ${hash}: ${rc.status}`);
  console.log(`  ${CHAINS[chainId].explorer}${hash}`);
  return rc.status === 'success';
}

const bigify = (o) => JSON.parse(JSON.stringify(o), (k, v) => (typeof v === 'string' && /^\d+$/.test(v) && k !== 'itemType' ? BigInt(v) : v));
const stringify = (o) => JSON.stringify(o, (_, v) => (typeof v === 'bigint' ? v.toString() : v), 2);

const [cmd, a1, a2, a3] = process.argv.slice(2);
const H = hoodi.id, B = baseSepolia.id;
if (cmd === 'address') {
  const h = client(H), b = client(B);
  const bal = (c, t) => c.readContract({ address: t, abi, functionName: 'balanceOf', args: [ME] });
  console.log(`${ME}`);
  console.log(`  Hoodi: ${formatEther(await h.getBalance({ address: ME }))} ETH, ${formatEther(await bal(h, LIDO.steth))} stETH, ${formatEther(await bal(h, LIDO.wsteth))} wstETH`);
  console.log(`  withdrawal requests: ${(await h.readContract({ address: LIDO.queue, abi, functionName: 'getWithdrawalRequests', args: [ME] })).join(', ') || 'none'}`);
  console.log(`  Base Sepolia: ${formatEther(await b.getBalance({ address: ME }))} ETH, ${await bal(b, TEST_NFT)} test NFTs`);
} else if (cmd === 'lido-stake') {
  console.log(`Lido: stake ${a1} ETH`);
  await send(H, LIDO.steth, encodeFunctionData({ abi, functionName: 'submit', args: [Z] }), parseEther(a1));
} else if (cmd === 'lido-wrap' || cmd === 'lido-unwrap') {
  const amount = parseEther(a1);
  if (cmd === 'lido-wrap') {
    console.log(`approve wstETH for ${a1} stETH`);
    if (!(await send(H, LIDO.steth, encodeFunctionData({ abi, functionName: 'approve', args: [LIDO.wsteth, amount] })))) process.exit(1);
  }
  console.log(`${cmd === 'lido-wrap' ? 'wrap' : 'unwrap'} ${a1}`);
  await send(H, LIDO.wsteth, encodeFunctionData({ abi, functionName: cmd === 'lido-wrap' ? 'wrap' : 'unwrap', args: [amount] }));
} else if (cmd === 'lido-withdraw' || cmd === 'lido-withdraw-to') {
  const amount = parseEther(a1);
  const owner = cmd === 'lido-withdraw' ? ME : a2;
  console.log(`approve the withdrawal queue for ${a1} stETH`);
  if (!(await send(H, LIDO.steth, encodeFunctionData({ abi, functionName: 'approve', args: [LIDO.queue, amount] })))) process.exit(1);
  console.log(`request a withdrawal of ${a1} stETH, owner ${owner}`);
  await send(H, LIDO.queue, encodeFunctionData({ abi, functionName: 'requestWithdrawals', args: [[amount], owner] }));
} else if (cmd === 'lido-claim') {
  console.log(`claim withdrawal ${a1}`);
  await send(H, LIDO.queue, encodeFunctionData({ abi, functionName: 'claimWithdrawal', args: [BigInt(a1)] }));
} else if (cmd === 'seaport-approve') {
  console.log(`let Seaport move the account's ${TEST_NFT} NFTs`);
  await send(B, TEST_NFT, encodeFunctionData({ abi, functionName: 'setApprovalForAll', args: [SEAPORT, true] }));
} else if (cmd === 'seaport-list') {
  const c = client(B);
  const id = BigInt(a1), price = parseEther(a2);
  const now = BigInt(Math.floor(Date.now() / 1000));
  const parameters = {
    offerer: ME, zone: Z,
    offer: [{ itemType: 2, token: TEST_NFT, identifierOrCriteria: id, startAmount: 1n, endAmount: 1n }],
    consideration: [{ itemType: 0, token: Z, identifierOrCriteria: 0n, startAmount: price, endAmount: price, recipient: ME }],
    orderType: 0, startTime: now - 60n, endTime: now + 7n * 86400n, zoneHash: Z32,
    salt: BigInt('0x' + crypto.randomBytes(16).toString('hex')), conduitKey: Z32,
    counter: await c.readContract({ address: SEAPORT, abi, functionName: 'getCounter', args: [ME] }),
  };
  console.log(`Seaport: list test NFT #${a1} for ${a2} ETH`);
  const it = (i) => ({ item_type: i.itemType, token: i.token, identifier: i.identifierOrCriteria.toString(), start_amount: i.startAmount.toString(), end_amount: i.endAmount.toString(), ...(i.recipient ? { recipient: i.recipient } : {}) });
  const sig = await execute({ type: 'seaport_order', order: {
    chain_id: B, seaport: SEAPORT, offerer: ME, zone: Z, offer: parameters.offer.map(it), consideration: parameters.consideration.map(it),
    order_type: 0, start_time: Number(parameters.startTime), end_time: Number(parameters.endTime), zone_hash: Z32,
    salt: parameters.salt.toString(), conduit_key: Z32, counter: parameters.counter.toString(),
  } });
  if (!sig) process.exit(1);
  const digest = hashTypedData({ domain: { name: 'Seaport', version: '1.6', chainId: B, verifyingContract: SEAPORT }, types: ORDER_TYPES, primaryType: 'OrderComponents', message: parameters });
  const signature = serializeSignature(rsv(sig));
  if ((await recoverAddress({ hash: digest, signature })).toLowerCase() !== ME.toLowerCase()) throw new Error('the order signature is not the account\'s');
  const file = a3 ?? `listing-${a1}.json`;
  fs.writeFileSync(file, stringify({ parameters: { ...parameters, totalOriginalConsiderationItems: parameters.consideration.length }, signature }));
  console.log(`  signed by the account; the order is in ${file} (order hash ${digest})`);
} else if (cmd === 'seaport-buy') {
  const { parameters: p, signature } = bigify(JSON.parse(fs.readFileSync(a1, 'utf8')));
  const [first, ...rest] = p.consideration;
  const value = p.consideration.reduce((t, c) => t + c.startAmount, 0n);
  console.log(`Seaport: buy ${p.offer[0].token} #${p.offer[0].identifierOrCriteria} from ${p.offerer} for ${formatEther(value)} ETH`);
  const data = encodeFunctionData({ abi, functionName: 'fulfillBasicOrder', args: [{
    considerationToken: Z, considerationIdentifier: 0n, considerationAmount: first.startAmount, offerer: p.offerer, zone: p.zone,
    offerToken: p.offer[0].token, offerIdentifier: p.offer[0].identifierOrCriteria, offerAmount: 1n, basicOrderType: 0,
    startTime: p.startTime, endTime: p.endTime, zoneHash: p.zoneHash, salt: p.salt, offererConduitKey: p.conduitKey, fulfillerConduitKey: Z32,
    totalOriginalAdditionalRecipients: BigInt(rest.length), additionalRecipients: rest.map((c) => ({ amount: c.startAmount, recipient: c.recipient })), signature,
  }] });
  await send(B, SEAPORT, data, value);
} else if (cmd === 'seaport-cancel') {
  const { parameters: p } = bigify(JSON.parse(fs.readFileSync(a1, 'utf8')));
  const { totalOriginalConsiderationItems, ...components } = p;
  console.log(`Seaport: cancel the listing of #${p.offer[0].identifierOrCriteria}`);
  await send(B, SEAPORT, encodeFunctionData({ abi, functionName: 'cancel', args: [[components]] }));
} else {
  console.log('usage: see the top of eth.mjs');
}
