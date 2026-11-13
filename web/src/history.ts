// Recent transactions per chain, from each chain's public explorer API
// (all allow browser requests). Chains without a free history API show an
// explorer link instead.

export type HistoryItem = { hash: string; time?: number; label: string; link: string; ok?: boolean };

const j = (url: string, init?: RequestInit) => fetch(url, { ...init, signal: AbortSignal.timeout(10000) }).then((r) => r.json());
const post = (url: string, body: unknown) => j(url, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });

/** Blockscout (v2 API) per EVM chain ID. */
const BLOCKSCOUT: Record<number, string> = {
  84532: 'https://base-sepolia.blockscout.com',
  11155111: 'https://eth-sepolia.blockscout.com',
  421614: 'https://arbitrum-sepolia.blockscout.com',
  11155420: 'https://optimism-sepolia.blockscout.com',
};
/** Esplora APIs (mempool.space's), with their explorer's tx page. */
const ESPLORA: Record<string, [string, string]> = {
  bitcoin: ['https://mempool.space/testnet4/api', 'https://mempool.space/testnet4/tx/'],
  litecoin: ['https://litecoinspace.org/testnet/api', 'https://litecoinspace.org/testnet/tx/'],
};

const fmt = (n: bigint, d: number, shown = 4) => {
  const neg = n < 0n;
  const a = neg ? -n : n;
  const p = 10n ** BigInt(d);
  const frac = ((a % p) * 10n ** BigInt(shown)) / p;
  return `${neg ? '−' : ''}${a / p}.${frac.toString().padStart(shown, '0')}`;
};

export function hasHistory(chain: string): boolean {
  const id = Number(chain);
  return chain === 'near' || chain in ESPLORA || ['solana', 'tron', 'xrp', 'ton', 'stellar'].includes(chain) || (!Number.isNaN(id) && id in BLOCKSCOUT);
}

export async function history(chain: string, address: string, wallet: string): Promise<HistoryItem[]> {
  if (chain === 'near') {
    const r = await j(`https://api-testnet.nearblocks.io/v3/accounts/${encodeURIComponent(wallet)}/txns?limit=15`);
    return (r.data ?? []).map((t: any) => {
      const methods = (t.actions ?? []).map((a: any) => a.method ?? a.action).filter(Boolean);
      const transfer = (t.actions ?? []).some((a: any) => a.action === 'TRANSFER');
      const incoming = t.receiver_account_id === wallet && t.signer_account_id !== wallet;
      const label = transfer
        ? `${incoming ? 'Received' : 'Sent'} ${fmt(BigInt(t.actions_agg?.deposit ?? '0'), 24)} NEAR`
        : methods.includes('w_execute_signed') ? 'Approved with your passkey'
        : methods.includes('w_execute_extension') ? 'An agent acted within its limits'
        : methods.join(', ') || 'Transaction';
      return { hash: t.transaction_hash, time: Number(BigInt(t.block_timestamp) / 1_000_000n), label, link: `https://testnet.nearblocks.io/txns/${t.transaction_hash}`, ok: t.outcomes?.status !== false };
    });
  }
  const evm = Number(chain);
  if (!Number.isNaN(evm) && BLOCKSCOUT[evm]) {
    const base = BLOCKSCOUT[evm];
    const r = await j(`${base}/api/v2/addresses/${address}/transactions`);
    return (r.items ?? []).slice(0, 15).map((t: any) => {
      const out = t.from?.hash?.toLowerCase() === address.toLowerCase();
      const v = BigInt(t.value ?? '0');
      const m = t.method ? String(t.method).replace(/^./, (c: string) => c.toUpperCase()) : '';
      const what = m ? m : v > 0n ? `${out ? 'Sent' : 'Received'} ${fmt(v, 18)}` : 'Transaction';
      const label = t.method && v > 0n ? `${what} · ${fmt(v, 18)}` : out ? what : `${what} (incoming)`;
      return { hash: t.hash, time: Date.parse(t.timestamp), label, link: `${base}/tx/${t.hash}`, ok: t.status !== 'error' };
    });
  }
  if (chain in ESPLORA) {
    const [api, tx] = ESPLORA[chain];
    const sym = chain === 'bitcoin' ? 'BTC' : 'LTC';
    const r: any[] = await j(`${api}/address/${address}/txs`);
    return r.slice(0, 15).map((t) => {
      const got = t.vout.filter((o: any) => o.scriptpubkey_address === address).reduce((s: bigint, o: any) => s + BigInt(o.value), 0n);
      const spent = t.vin.filter((i: any) => i.prevout?.scriptpubkey_address === address).reduce((s: bigint, i: any) => s + BigInt(i.prevout.value), 0n);
      const net: bigint = (got as bigint) - (spent as bigint);
      return { hash: t.txid, time: t.status?.block_time ? t.status.block_time * 1000 : undefined, label: `${net >= 0n ? 'Received' : 'Sent'} ${fmt(net < 0n ? -net : net, 8)} ${sym}${t.status?.confirmed ? '' : ' · pending'}`, link: tx + t.txid };
    });
  }
  if (chain === 'solana') {
    const r = await post('https://api.devnet.solana.com', { jsonrpc: '2.0', id: 1, method: 'getSignaturesForAddress', params: [address, { limit: 15 }] });
    return (r.result ?? []).map((s: any) => ({ hash: s.signature, time: s.blockTime ? s.blockTime * 1000 : undefined, label: s.err ? 'Failed transaction' : 'Transaction', link: `https://explorer.solana.com/tx/${s.signature}?cluster=devnet`, ok: !s.err }));
  }
  if (chain === 'tron') {
    const r = await j(`https://nile.trongrid.io/v1/accounts/${address}/transactions?limit=15`);
    return (r.data ?? []).map((t: any) => {
      const c = t.raw_data?.contract?.[0];
      const v = c?.parameter?.value ?? {};
      const amount = v.amount !== undefined ? ` ${fmt(BigInt(v.amount), 6)} TRX` : '';
      return { hash: t.txID, time: t.block_timestamp, label: c?.type === 'TransferContract' ? `Transfer${amount}` : (c?.type ?? 'Transaction'), link: `https://nile.tronscan.org/#/transaction/${t.txID}`, ok: t.ret?.[0]?.contractRet === 'SUCCESS' };
    });
  }
  if (chain === 'xrp') {
    const r = await post('https://testnet.xrpl-labs.com', { method: 'account_tx', params: [{ account: address, limit: 15 }] });
    return (r.result?.transactions ?? []).map((e: any) => {
      const t = e.tx ?? e.tx_json ?? {};
      const hash = t.hash ?? e.hash;
      const amt = typeof (e.meta?.delivered_amount ?? t.Amount) === 'string' ? ` ${fmt(BigInt(e.meta?.delivered_amount ?? t.Amount), 6)} XRP` : '';
      const out = t.Account === address;
      const label = t.TransactionType === 'Payment' ? `${out ? 'Sent' : 'Received'}${amt}` : (t.TransactionType ?? 'Transaction');
      const time = t.date !== undefined ? (t.date + 946_684_800) * 1000 : undefined;
      return { hash, time, label, link: `https://testnet.xrpl.org/transactions/${hash}`, ok: e.meta?.TransactionResult === 'tesSUCCESS' };
    });
  }
  if (chain === 'stellar') {
    const r = await fetch(`https://horizon-testnet.stellar.org/accounts/${address}/operations?order=desc&limit=15`, { signal: AbortSignal.timeout(10000) });
    if (r.status === 404) return [];
    const j = await r.json();
    return (j._embedded?.records ?? []).map((o: any) => {
      const amt = o.amount ?? o.starting_balance;
      const code = o.asset_type === 'native' || o.type === 'create_account' ? 'XLM' : (o.asset_code ?? '');
      const dir = o.to === address || o.account === address ? 'Received' : 'Sent';
      const label = o.type === 'payment' || o.type === 'create_account' ? `${dir} ${Number(amt).toFixed(4)} ${code}` : o.type.replace(/_/g, ' ');
      return { hash: o.transaction_hash, time: Date.parse(o.created_at), label, link: `https://stellar.expert/explorer/testnet/tx/${o.transaction_hash}`, ok: o.transaction_successful };
    });
  }
  if (chain === 'ton') {
    const r = await j(`https://testnet.toncenter.com/api/v3/transactions?account=${encodeURIComponent(address)}&limit=15&sort=desc`);
    return (r.transactions ?? []).map((t: any) => {
      const inVal = BigInt(t.in_msg?.value ?? 0);
      const outs = (t.out_msgs ?? []) as any[];
      const outVal = outs.reduce((a, m) => a + BigInt(m.value ?? 0), 0n);
      const label = outs.length ? `Sent ${fmt(outVal, 9)} GRAM` : inVal > 0n ? `Received ${fmt(inVal, 9)} GRAM` : 'Transaction';
      const ok = !t.description?.aborted && (t.description?.compute_ph?.success ?? true);
      // TON Center gives the hash in base64; the explorer takes hex.
      const hex = Array.from(atob(t.hash), (c) => c.charCodeAt(0).toString(16).padStart(2, '0')).join('');
      return { hash: hex, time: t.now * 1000, label, link: `https://testnet.tonviewer.com/transaction/${hex}`, ok };
    });
  }
  return [];
}
