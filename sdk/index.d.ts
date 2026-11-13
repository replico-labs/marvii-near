export declare const DEFAULTS: { page: string; relayer: string };

export interface SessionKey { seed_hex: string; public_key_hex: string }
/** A new ed25519 session key. Keep `seed_hex` secret. */
export declare function generateKey(): SessionKey;

export interface ActionToSign { extension: string; wallet: string; keyId: string; nonce: number; action: Action }
export interface Signer { signAction(m: ActionToSign): Promise<{ msg: string; signature: string }> }

/** Signs with a session key held by this process. */
export declare class LocalSigner implements Signer {
  constructor(seedHex: string);
  readonly publicKey: string;
  signAction(m: ActionToSign): Promise<{ msg: string; signature: string }>;
}

/** Signs inside OutLayer's TEE; the key never leaves the enclave. */
export declare class OutLayerSigner implements Signer {
  constructor(o: {
    moduleUrl: string; moduleHash: string;
    operator?: { accountId: string; privateKey: string };
    run?: (input: object) => Promise<any>;
    networkId?: string; rpc?: string; outlayer?: string;
  });
  getPublicKey(): Promise<string>;
  /** After getPublicKey(): the NEAR transaction of the OutLayer run that made the key. */
  keyProof?: { tx_hash: string; sender: string };
  signAction(m: ActionToSign): Promise<{ msg: string; signature: string }>;
}

/** The extension's Grant fields an app may ask for (see near/extension/src/rules.rs). */
export interface GrantRequest {
  near_venues?: { receiver_id: string; method: string }[];
  evm_venues?: { chain_id: number; to: string; selector: string }[];
  near_recipients?: string[];
  evm_recipients?: string[];
  caps?: { asset: string; daily: string }[];
  queue_new_targets?: boolean;
  hl_markets?: { asset: number; max_leverage: number }[];
  evm_self?: string;
  evm_nft_floor?: string;
  utxo?: { chain: string; self_script: string; recipients: string[] }[];
  sol?: object; xrpl?: object; stellar?: object; tron?: object; ton?: object;
}

/** The link the owner opens to approve the agent with their passkey. */
export declare function permissionLink(r: {
  publicKey: string; keyId: string; app: { name: string; url?: string }; grant: GrantRequest;
  days?: number; idleDays?: number; returnUrl?: string; page?: string;
  /** `signer.keyProof` from an OutLayerSigner: the page approves only keys made in Marvii's published module. */
  keyProof?: { tx_hash: string; sender: string };
}): string;

/** Waits until a wallet has granted `publicKey`; the relayer checks it on-chain. */
export declare function waitForApproval(o: { publicKey: string; relayer?: string; timeoutMs?: number; intervalMs?: number }): Promise<{ wallet: string; key_id: string; extension: string }>;

export type Action = { type: string; [k: string]: unknown };
export interface Outcome {
  ok: boolean; tx_hash?: string; reason?: string | null; error?: string;
  mpc_signature?: unknown; pending_id?: number; evm_signing_hash?: string;
}

/** An agent acting for one wallet through one session key. */
export declare class MarviiAgent {
  constructor(o: { wallet: string; keyId: string; signer: Signer; relayer?: string });
  view<T = any>(contractId: string, method: string, args?: object): Promise<T>;
  session(): Promise<any | null>;
  spentToday(asset: string): Promise<bigint>;
  address(path: 'evm' | 'solana' | 'ton' | 'stellar' | 'tron' | 'xrpl'): Promise<string | undefined>;
  execute(action: Action): Promise<Outcome>;
  propose(action: Action, o?: { wait?: boolean }): Promise<Outcome & { queue_id?: number; ready_at_ms?: number }>;
  waitForOwner(pendingId: number, o?: { timeoutMs?: number }): Promise<{ status: 'approved' | 'rejected'; mpc_signature?: unknown }>;
  topUpQuote(o: { from: string; to: string; amount: string | bigint; refundTo: string; recipient: string; slippageBps?: number }): Promise<TopUpQuote>;
  topUpSubmit(txHash: string, depositAddress: string): Promise<{ status?: string }>;
  topUpStatus(depositAddress: string): Promise<{ status: string; amount_out?: string; destination_tx?: unknown }>;
}

export declare function nearCall(receiverId: string, method: string, args?: object, deposit?: string | bigint): Action;
export declare function evmTx(tx: object): Action;
export declare function solTx(o: { recentBlockhash: string; instructions: object[]; atas?: object[]; obligations?: object[]; traders?: { sub: number; bump: number }[] }): Action;
export declare function phoenixOrder(o: { sub: number; bump: number; recentBlockhash: string; instructions: object[] }): Action;
export declare function tonTx(tx: object): Action;
export declare function tronTx(tx: object): Action;
export declare function stellarTx(tx: object): Action;
export declare function xrplTx(tx: object): Action;
export declare function seaportOrder(order: object): Action;
export declare function utxoPay(p: object): Action;
export interface TopUpQuote {
  /** The exact JSON 1Click signed, and its signature. */
  quote: string;
  signature: string;
  deposit_address: string;
  amount_in: string;
  amount_out: string;
  min_amount_out: string;
  amount_in_usd: string;
  amount_out_usd: string;
  deadline: string;
  time_estimate_s: number;
}
export declare function polymarketOrder(order: { exchange: string; salt: string; maker: string; signer: string; taker: string; token_id: string; maker_amount: string; taker_amount: string; expiration: number; nonce: string; fee_rate_bps: string; side: 0 | 1; signature_type: 0 }): Action;
export declare function intentsDeposit(q: { quote: string; signature: string }, action: Action): Action;
export declare function hlOrder(o: object): Action;
export declare function hlCancel(o: object): Action;
export declare function hlLeverage(o: object): Action;
