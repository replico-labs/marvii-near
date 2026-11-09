// Read-only NEAR RPC calls. Anything that changes state goes through the
// wallet (passkey) or the extension (session rules), via the relayer.

export type Config = { chain_id: string; rpc: string; signer: string; extension: string; wallet_code_hash: string; names?: string | null };

export const RELAYER = (import.meta.env.VITE_RELAYER as string | undefined) ?? 'http://localhost:8787';

let rpcUrl = '';
export function setRpc(url: string) {
  rpcUrl = url;
}

async function rpc(method: string, params: unknown) {
  const res = await fetch(rpcUrl, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
  });
  const j = await res.json();
  if (j.error) throw new Error(j.error.cause?.name ?? j.error.message);
  return j.result;
}

export async function account(id: string): Promise<{ amount: string; code_hash: string; global_contract_hash?: string } | null> {
  try {
    return await rpc('query', { request_type: 'view_account', finality: 'final', account_id: id });
  } catch (e) {
    if (String(e).includes('UNKNOWN_ACCOUNT')) return null;
    throw e;
  }
}

export async function view<T>(contract: string, method: string, args: unknown): Promise<T> {
  const r = await rpc('query', {
    request_type: 'call_function',
    finality: 'final',
    account_id: contract,
    method_name: method,
    args_base64: btoa(JSON.stringify(args)),
  });
  return JSON.parse(new TextDecoder().decode(new Uint8Array(r.result)));
}

export async function relayer<T>(path: string, body?: unknown): Promise<T> {
  const res = await fetch(RELAYER + path, body === undefined ? {} : {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
  const j = await res.json();
  if (!res.ok) throw new Error(j.error ?? res.statusText);
  return j;
}

export const formatNear = (yocto: string) => {
  const n = BigInt(yocto);
  const whole = n / 10n ** 24n;
  const frac = ((n % 10n ** 24n) / 10n ** 20n).toString().padStart(4, '0');
  return `${whole}.${frac}`;
};
