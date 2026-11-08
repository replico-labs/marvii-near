/* tslint:disable */
/* eslint-disable */

/**
 * Builds the message for one function call from the wallet, with the
 * wallet's own types, so the page never assembles the JSON by hand.
 * `created_at_secs` should be about a minute before now (the wallet
 * refuses messages from the future).
 */
export function call_message(chain_id: string, wallet: string, nonce: number, created_at_secs: bigint, receiver: string, method: string, args_json: string, deposit_yocto: string, gas_tgas: bigint): string;

/**
 * Compresses a SEC1 P-256 public key (65 bytes uncompressed or 33
 * compressed), as hex. Checks it's a point on the curve.
 */
export function compress_public_key(sec1_hex: string): string;

/**
 * Decodes an EVM transaction for the Confirm screen and computes the hash
 * the passkey will approve. Unknown functions are shown as such, never
 * guessed.
 */
export function describe_evm_tx(tx_json: string): string;

/**
 * Decodes a message into what the Confirm screen shows: who signs, until
 * when, every wallet op and every call with its decoded arguments. It
 * works from the parsed message, which is exactly what gets hashed.
 */
export function describe_message(msg_json: string): string;

/**
 * An empty request: the first one deploys the wallet (the relayer
 * attaches the state init) without doing anything else.
 */
export function empty_message(chain_id: string, wallet: string, nonce: number, created_at_secs: bigint): string;

/**
 * The EVM address for an MPC derived key (`secp256k1:<base58 of x||y>`,
 * as the signer's `derived_public_key` view returns it):
 * the last 20 bytes of `keccak256(x||y)`.
 */
export function evm_address(derived_public_key: string): string;

/**
 * Builds the call data and transaction fields for an owner's send: the
 * native coin when `token` is empty, else ERC-20 `transfer`.
 */
export function evm_send_tx(chain_id: bigint, nonce: bigint, max_priority_fee_per_gas: string, max_fee_per_gas: string, gas_limit: bigint, token: string, to: string, amount: string): string;

/**
 * The signed EIP-1559 transaction for an MPC signature
 * (`{ big_r: { affine_point }, s: { scalar }, recovery_id }`, as the
 * signer returns it). S is normalised to low S, as Ethereum requires. The
 * sender is recovered here, so the page broadcasts only a transaction
 * signed by the address it expects.
 */
export function evm_signed_tx(tx_json: string, mpc_signature_json: string): string;

/**
 * Builds the proof `w_execute_signed` expects from a WebAuthn assertion.
 * The DER signature is converted to r||s and normalised to low S, which
 * the wallet requires.
 */
export function make_proof(authenticator_data_hex: string, client_data_json: string, der_signature_hex: string): string;

/**
 * Parses a request message (the wallet's JSON form) and returns its
 * canonical JSON back, so the page shows and signs the same parsed value.
 */
export function normalize_request(msg_json: string): string;

/**
 * The message by which the passkey has the wallet itself ask the MPC
 * signer to sign this EVM transaction, on the wallet's one EVM path
 * (`evm`). The hash is computed here from the transaction, never taken
 * from anyone else.
 */
export function owner_evm_sign_message(chain_id: string, wallet: string, nonce: number, created_at_secs: bigint, signer: string, tx_json: string): string;

/**
 * The 32 bytes the passkey signs as its WebAuthn challenge:
 * `SHA3-256("NEAR_WALLET_CONTRACT/V1" || borsh(msg))`, as hex.
 */
export function request_hash(msg_json: string): string;

/**
 * Checks a proof against a public key and message, with the wallet's own
 * verifier. The page runs this before sending, so a bad proof never
 * reaches the relayer.
 */
export function verify_proof(public_key_hex: string, msg_json: string, proof: string): boolean;

/**
 * The wallet's account ID (`0s…`), known before it exists on-chain.
 */
export function wallet_account_id(code_hash_hex: string, public_key_hex: string, extension: string): string;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly call_message: (a: number, b: number, c: number, d: number, e: number, f: bigint, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: bigint) => [number, number, number, number];
    readonly compress_public_key: (a: number, b: number) => [number, number, number, number];
    readonly describe_evm_tx: (a: number, b: number) => [number, number, number, number];
    readonly describe_message: (a: number, b: number) => [number, number, number, number];
    readonly empty_message: (a: number, b: number, c: number, d: number, e: number, f: bigint) => [number, number, number, number];
    readonly evm_address: (a: number, b: number) => [number, number, number, number];
    readonly evm_send_tx: (a: bigint, b: bigint, c: number, d: number, e: number, f: number, g: bigint, h: number, i: number, j: number, k: number, l: number, m: number) => [number, number, number, number];
    readonly evm_signed_tx: (a: number, b: number, c: number, d: number) => [number, number, number, number];
    readonly make_proof: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number, number, number];
    readonly normalize_request: (a: number, b: number) => [number, number, number, number];
    readonly owner_evm_sign_message: (a: number, b: number, c: number, d: number, e: number, f: bigint, g: number, h: number, i: number, j: number) => [number, number, number, number];
    readonly request_hash: (a: number, b: number) => [number, number, number, number];
    readonly verify_proof: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number, number];
    readonly wallet_account_id: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number, number, number];
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
