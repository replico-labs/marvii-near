/* tslint:disable */
/* eslint-disable */

/**
 * Plans a send: picks coins (largest first) from `utxos_json`
 * (`[{txid, vout, value}]`, as mempool.space lists them), pays `to`, and
 * returns change to `from` unless it would be dust. At most 6 inputs.
 * `from` picks the chain (Bitcoin testnet4 or Litecoin testnet).
 */
export function bitcoin_plan(from: string, to: string, amount_sat: string, utxos_json: string, fee_rate_sat_vb: number): string;

/**
 * The signed transaction (hex) and its txid. Each MPC signature is matched
 * to the input whose hash it signs, and must recover to the key behind
 * `from`; S is made low and DER-encoded.
 */
export function bitcoin_signed_tx(plan_json: string, mpc_signatures_json: string): string;

/**
 * Builds the message for one function call from the wallet, with the
 * wallet's own types, so the page never assembles the JSON by hand.
 * `created_at_secs` should be about a minute before now (the wallet
 * refuses messages from the future).
 */
export function call_message(chain_id: string, wallet: string, nonce: number, created_at_secs: bigint, receiver: string, method: string, args_json: string, deposit_yocto: string, gas_tgas: bigint): string;

/**
 * Plans an ADA send: coins largest first (only ADA-only UTxOs; Koios's
 * `[{tx_hash, tx_index, value, asset_list}]`), change back to `from`
 * unless below the minimum ADA, then it goes to the fee. `ttl` is the
 * last slot the transaction is valid in.
 */
export function cardano_plan(from: string, to: string, lovelace: string, utxos_json: string, ttl: bigint): string;

/**
 * The signed transaction (CBOR hex, for submission) and its txid, after
 * checking the MPC signature (`{ signature: [64] }`) verifies over the body
 * hash for a key whose hash is `from`'s payment key hash.
 */
export function cardano_signed_tx(plan_json: string, mpc_signature_json: string, derived_public_key: string): string;

/**
 * The account's address on a non-EVM chain, from the MPC signer's derived
 * key for that chain's path (`secp256k1:<base58 x||y>` or
 * `ed25519:<base58>`). Testnet encodings; each is checked against the
 * chain's own reference library in the tests.
 */
export function chain_address(chain: string, derived_public_key: string): string;

/**
 * The derivation path and key domain each chain uses: one path per chain,
 * `domain_id` 0 (secp256k1) or 1 (ed25519).
 */
export function chain_path(chain: string): string;

/**
 * Compresses a SEC1 P-256 public key (65 bytes uncompressed or 33
 * compressed), as hex. Checks it's a point on the curve.
 */
export function compress_public_key(sec1_hex: string): string;

/**
 * For Confirm: what a plan pays, checked again from its parts (the
 * signing hashes are recomputed here, not trusted).
 */
export function describe_bitcoin_plan(plan_json: string): string;

/**
 * For Confirm: what a plan pays, with the body hash recomputed.
 */
export function describe_cardano_plan(plan_json: string): string;

/**
 * Decodes an EVM transaction for the Confirm screen and computes the hash
 * the passkey will approve. Unknown functions are shown as such, never
 * guessed.
 */
export function describe_evm_tx(tx_json: string): string;

/**
 * For Confirm: what a plan pays, with the signing hashes recomputed.
 */
export function describe_legacy_plan(plan_json: string): string;

/**
 * Decodes a message into what the Confirm screen shows: who signs, until
 * when, every wallet op and every call with its decoded arguments. It
 * works from the parsed message, which is exactly what gets hashed.
 */
export function describe_message(msg_json: string): string;

/**
 * Decodes a message for Confirm. Only the exact single-transfer layout
 * above is recognised; anything else is refused, never guessed.
 */
export function describe_solana_message(message_hex: string): string;

/**
 * Decodes a send for Confirm by re-encoding it from its fields: only the
 * exact layout stellar_payment builds is recognised.
 */
export function describe_stellar(tx_hex: string): string;

/**
 * Decodes a signing message for Confirm: only the exact one-payment layout
 * ton_transfer builds is recognised, by rebuilding it from its fields.
 */
export function describe_ton(signing_boc_hex: string): string;

/**
 * For Confirm: decodes a raw transaction, accepting only the exact
 * single-TRX-transfer layout built above.
 */
export function describe_tron_raw(raw_hex: string): string;

/**
 * For Confirm: decodes an unsigned Payment, accepting only the exact layout
 * built above (re-encoded and compared byte for byte).
 */
export function describe_xrp_tx(tx_hex: string): string;

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
 * The passkey moves the wallet to another extension: adds `add`, removes
 * each of `remove_json` (a JSON list). The address doesn't change. Sessions
 * live in each extension, so the old one's sessions stop working.
 */
export function extension_move_message(chain_id: string, wallet: string, nonce: number, created_at_secs: bigint, add: string, remove_json: string): string;

/**
 * The exact action Hyperliquid receives, for the page to send.
 */
export function hl_transfer_action(amount: string, to_perp: boolean, nonce: bigint): string;

/**
 * The EIP-712 digest of a spot↔perps USDC transfer on Hyperliquid testnet.
 */
export function hl_transfer_hash(amount: string, to_perp: boolean, nonce: bigint): string;

/**
 * Plans a send on Dogecoin, Bitcoin Cash or Zcash: coins largest first
 * (at most 6), change back to `from` unless it's dust. `fee_rate` is per
 * byte (Dogecoin, Bitcoin Cash); Zcash's fee is ZIP-317's, 5,000 zatoshis
 * per logical action with a minimum of two. `branch_id` and
 * `expiry_height` are for Zcash only (0 otherwise).
 */
export function legacy_plan(chain: string, from: string, to: string, amount: string, utxos_json: string, fee_rate: number, branch_id: number, expiry_height: number): string;

/**
 * The output script (hex) for a testnet address on Dogecoin, Bitcoin Cash
 * or Zcash; the relayer derives Electrum script hashes from it.
 */
export function legacy_script_pubkey(chain: string, address: string): string;

/**
 * The signed transaction (hex) and its txid, after every signature is
 * checked against the account's key.
 */
export function legacy_signed_tx(plan_json: string, mpc_signatures_json: string): string;

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
 * The passkey's request for one ECDSA signature per hash, on a chain's
 * path: one call to the signer per hash.
 */
export function owner_ecdsa_multi_sign_message(chain_id: string, wallet: string, nonce: number, created_at_secs: bigint, signer: string, chain: string, hashes_json: string): string;

/**
 * The message by which the passkey has the wallet ask the MPC signer for
 * an ed25519 signature over `message_hex`, on the given chain's path.
 */
export function owner_eddsa_sign_message(chain_id: string, wallet: string, nonce: number, created_at_secs: bigint, signer: string, chain: string, message_hex: string): string;

/**
 * The message by which the passkey has the wallet itself ask the MPC
 * signer to sign this EVM transaction, on the wallet's one EVM path
 * (`evm`). The hash is computed here from the transaction, never taken
 * from anyone else.
 */
export function owner_evm_sign_message(chain_id: string, wallet: string, nonce: number, created_at_secs: bigint, signer: string, tx_json: string): string;

/**
 * The passkey's request: the wallet asks the MPC signer to sign that
 * digest with its EVM key (path `evm`).
 */
export function owner_hl_transfer_message(chain_id: string, wallet: string, nonce: number, created_at_secs: bigint, signer: string, amount: string, to_perp: boolean, hl_nonce: bigint): string;

/**
 * The 32 bytes the passkey signs as its WebAuthn challenge:
 * `SHA3-256("NEAR_WALLET_CONTRACT/V1" || borsh(msg))`, as hex.
 */
export function request_hash(msg_json: string): string;

/**
 * The signed Solana transaction (base64, for `sendTransaction`), after
 * checking the MPC signature (`{ scheme: "Ed25519", signature: [64] }`)
 * verifies for `from` over exactly this message.
 */
export function solana_signed_tx(message_hex: string, mpc_signature_json: string, from: string): string;

/**
 * A legacy Solana message for one System Program transfer, as hex: the
 * exact bytes the account's ed25519 key signs.
 */
export function solana_transfer_message(from: string, to: string, lamports: string, recent_blockhash: string): string;

/**
 * An XLM send from the owner: the transaction XDR (hex) and its hash,
 * which the MPC key signs. `create` when the recipient doesn't exist yet
 * (at least 1 XLM, Stellar's minimum balance).
 */
export function stellar_payment(derived_public_key: string, to: string, stroops: string, create: boolean, fee: number, seq_num: string, max_time: number, memo: string): string;

/**
 * The signed envelope (base64, for Horizon's /transactions), after
 * checking the MPC signature verifies for the account over the hash.
 */
export function stellar_signed_tx(tx_hex: string, mpc_signature_json: string, derived_public_key: string): string;

/**
 * The external message (BOC, base64, for sendBoc) carrying a signed
 * signing message, after checking the MPC signature verifies for the
 * account's key over its hash. `deploy` adds the state init (seqno 0).
 */
export function ton_signed_tx(signing_boc_hex: string, mpc_signature_json: string, derived_public_key: string, deploy: boolean): string;

/**
 * One TON payment from the owner: the signing message (BOC hex) and its
 * hash, which the MPC key signs. Bounce follows the recipient's address
 * form (EQ/kQ bounceable, UQ/0Q not), as wallets do.
 */
export function ton_transfer(derived_public_key: string, to: string, nanotons: string, seqno: number, valid_until: number, comment: string): string;

/**
 * The owner sends NEAR: one transfer to `receiver` (any account, a name
 * included). Confirm shows it from the decoded message.
 */
export function transfer_message(chain_id: string, wallet: string, nonce: number, created_at_secs: bigint, receiver: string, yocto: string): string;

/**
 * The signed transaction as protobuf hex (for `broadcasthex`), after
 * checking the MPC signature over the txid recovers to `from`.
 */
export function tron_signed_tx(raw_hex: string, mpc_signature_json: string, from: string): string;

/**
 * The raw transaction for a TRX transfer, built here from the latest
 * block's reference (`ref_block_bytes` 2 bytes, `ref_block_hash` 8 bytes):
 * `{ raw_hex, txid }`, where txid = sha256(raw) is what gets signed.
 */
export function tron_transfer_raw(from: string, to: string, amount_sun: string, ref_block_bytes_hex: string, ref_block_hash_hex: string, expiration_ms: bigint, timestamp_ms: bigint): string;

/**
 * The testnet address an output script pays, for showing agent payment
 * grants and requests in words. Refuses scripts that aren't standard.
 */
export function utxo_address(chain: string, script_hex: string): string;

/**
 * The output script (hex) for a testnet address on any Bitcoin-family
 * chain here (SegWit too, for Bitcoin and Litecoin): what agent payment
 * grants and requests carry.
 */
export function utxo_script(chain: string, address: string): string;

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

/**
 * An XRP Payment built here: `{ tx_hex, signing_hash }`. The signing key
 * is the account's derived key (`secp256k1:<base58 x||y>`), and it must
 * hash to `from`. `destination_tag` is optional (pass a negative number for none).
 */
export function xrp_payment(from: string, derived_public_key: string, to: string, drops: string, fee_drops: string, sequence: number, last_ledger_sequence: number, destination_tag: number): string;

/**
 * The signed transaction blob (uppercase hex, for `submit`) and its hash,
 * after checking the MPC signature verifies under the transaction's own
 * signing key, which belongs to `from`. The signature is made low-S, as the
 * ledger requires.
 */
export function xrp_signed_tx(tx_hex: string, mpc_signature_json: string, from: string): string;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly bitcoin_plan: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number) => [number, number, number, number];
    readonly bitcoin_signed_tx: (a: number, b: number, c: number, d: number) => [number, number, number, number];
    readonly call_message: (a: number, b: number, c: number, d: number, e: number, f: bigint, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: bigint) => [number, number, number, number];
    readonly cardano_plan: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: bigint) => [number, number, number, number];
    readonly cardano_signed_tx: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number, number, number];
    readonly chain_address: (a: number, b: number, c: number, d: number) => [number, number, number, number];
    readonly chain_path: (a: number, b: number) => [number, number, number, number];
    readonly compress_public_key: (a: number, b: number) => [number, number, number, number];
    readonly describe_bitcoin_plan: (a: number, b: number) => [number, number, number, number];
    readonly describe_cardano_plan: (a: number, b: number) => [number, number, number, number];
    readonly describe_evm_tx: (a: number, b: number) => [number, number, number, number];
    readonly describe_legacy_plan: (a: number, b: number) => [number, number, number, number];
    readonly describe_message: (a: number, b: number) => [number, number, number, number];
    readonly describe_solana_message: (a: number, b: number) => [number, number, number, number];
    readonly describe_stellar: (a: number, b: number) => [number, number, number, number];
    readonly describe_ton: (a: number, b: number) => [number, number, number, number];
    readonly describe_tron_raw: (a: number, b: number) => [number, number, number, number];
    readonly describe_xrp_tx: (a: number, b: number) => [number, number, number, number];
    readonly empty_message: (a: number, b: number, c: number, d: number, e: number, f: bigint) => [number, number, number, number];
    readonly evm_address: (a: number, b: number) => [number, number, number, number];
    readonly evm_send_tx: (a: bigint, b: bigint, c: number, d: number, e: number, f: number, g: bigint, h: number, i: number, j: number, k: number, l: number, m: number) => [number, number, number, number];
    readonly evm_signed_tx: (a: number, b: number, c: number, d: number) => [number, number, number, number];
    readonly extension_move_message: (a: number, b: number, c: number, d: number, e: number, f: bigint, g: number, h: number, i: number, j: number) => [number, number, number, number];
    readonly hl_transfer_action: (a: number, b: number, c: number, d: bigint) => [number, number, number, number];
    readonly hl_transfer_hash: (a: number, b: number, c: number, d: bigint) => [number, number, number, number];
    readonly legacy_plan: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number) => [number, number, number, number];
    readonly legacy_script_pubkey: (a: number, b: number, c: number, d: number) => [number, number, number, number];
    readonly legacy_signed_tx: (a: number, b: number, c: number, d: number) => [number, number, number, number];
    readonly make_proof: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number, number, number];
    readonly normalize_request: (a: number, b: number) => [number, number, number, number];
    readonly owner_ecdsa_multi_sign_message: (a: number, b: number, c: number, d: number, e: number, f: bigint, g: number, h: number, i: number, j: number, k: number, l: number) => [number, number, number, number];
    readonly owner_eddsa_sign_message: (a: number, b: number, c: number, d: number, e: number, f: bigint, g: number, h: number, i: number, j: number, k: number, l: number) => [number, number, number, number];
    readonly owner_evm_sign_message: (a: number, b: number, c: number, d: number, e: number, f: bigint, g: number, h: number, i: number, j: number) => [number, number, number, number];
    readonly owner_hl_transfer_message: (a: number, b: number, c: number, d: number, e: number, f: bigint, g: number, h: number, i: number, j: number, k: number, l: bigint) => [number, number, number, number];
    readonly request_hash: (a: number, b: number) => [number, number, number, number];
    readonly solana_signed_tx: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number, number, number];
    readonly solana_transfer_message: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => [number, number, number, number];
    readonly stellar_payment: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number) => [number, number, number, number];
    readonly stellar_signed_tx: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number, number, number];
    readonly ton_signed_tx: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => [number, number, number, number];
    readonly ton_transfer: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number) => [number, number, number, number];
    readonly transfer_message: (a: number, b: number, c: number, d: number, e: number, f: bigint, g: number, h: number, i: number, j: number) => [number, number, number, number];
    readonly tron_signed_tx: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number, number, number];
    readonly tron_transfer_raw: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: bigint, l: bigint) => [number, number, number, number];
    readonly utxo_address: (a: number, b: number, c: number, d: number) => [number, number, number, number];
    readonly utxo_script: (a: number, b: number, c: number, d: number) => [number, number, number, number];
    readonly verify_proof: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number, number];
    readonly wallet_account_id: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number, number, number];
    readonly xrp_payment: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number) => [number, number, number, number];
    readonly xrp_signed_tx: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number, number, number];
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
