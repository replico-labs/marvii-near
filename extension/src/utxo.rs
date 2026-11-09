//! Agent payments on payment-only chains: Bitcoin and Litecoin (native
//! SegWit, BIP-143), Bitcoin Cash (BIP-143 with FORKID) and Zcash
//! (transparent, ZIP-244).
//!
//! The agent sends inputs and outputs; the extension builds every signing
//! hash itself. Each of these digests commits to the amounts of the coins
//! being spent, so an agent that misstates an input gets a signature the
//! chain rejects: the totals the extension counts are the real ones, fee
//! included. Dogecoin's legacy digest doesn't commit to amounts, so it isn't
//! offered to agents.
//!
//! Must match `near/core` (the page's and agent's transaction assembly):
//! the same versions, sequences and lock time.

use near_sdk::env;

pub const CHAINS: [&str; 4] = ["bitcoin", "litecoin", "bitcoin-cash", "zcash"];
pub const MAX_INPUTS: usize = 2;
pub const MAX_OUTPUTS: usize = 3;

const SEQ_SEGWIT: u32 = 0xffff_fffd; // replaceable, as core's Bitcoin plans
const SEQ_FINAL: u32 = 0xffff_ffff; // core's Bitcoin Cash and Zcash plans
const ZEC_V5_HEADER: u32 = 0x8000_0005;
const ZEC_V5_GROUP: u32 = 0x26A7_270A;

pub struct Input {
    /// Display order (as explorers show it), 32 bytes.
    pub txid: [u8; 32],
    pub vout: u32,
    pub value: u64,
}

pub struct Output {
    pub script: Vec<u8>,
    pub value: u64,
}

/// The account's own script for this chain must be a P2WPKH program
/// (Bitcoin, Litecoin) or a P2PKH script (Bitcoin Cash, Zcash).
pub fn self_script_ok(chain: &str, s: &[u8]) -> bool {
    match chain {
        "bitcoin" | "litecoin" => s.len() == 22 && s[0] == 0x00 && s[1] == 0x14,
        "bitcoin-cash" | "zcash" => s.len() == 25 && s[..3] == [0x76, 0xa9, 0x14] && s[23..] == [0x88, 0xac],
        _ => false,
    }
}

/// A recipient script of a standard shape: P2PKH, P2SH, or (Bitcoin and
/// Litecoin only) a SegWit program.
pub fn recipient_script_ok(chain: &str, s: &[u8]) -> bool {
    let p2pkh = s.len() == 25 && s[..3] == [0x76, 0xa9, 0x14] && s[23..] == [0x88, 0xac];
    let p2sh = s.len() == 23 && s[..2] == [0xa9, 0x14] && s[22] == 0x87;
    let segwit = s.len() >= 4 && s.len() <= 42 && (s[0] == 0 || (0x51..=0x60).contains(&s[0])) && s[1] as usize == s.len() - 2;
    p2pkh || p2sh || (matches!(chain, "bitcoin" | "litecoin") && segwit)
}

fn sha256d(b: &[u8]) -> [u8; 32] {
    env::sha256_array(&env::sha256_array(b))
}

fn varint(out: &mut Vec<u8>, n: usize) {
    // Scripts and counts here stay far below 0xfd.
    out.push(n as u8);
}

fn outpoint(out: &mut Vec<u8>, i: &Input) {
    let mut t = i.txid;
    t.reverse();
    out.extend(t);
    out.extend(i.vout.to_le_bytes());
}

fn outputs_bytes(outputs: &[Output]) -> Vec<u8> {
    let mut o = vec![];
    for x in outputs {
        o.extend(x.value.to_le_bytes());
        varint(&mut o, x.script.len());
        o.extend(&x.script);
    }
    o
}

fn blake(personal: &[u8; 16], data: &[u8]) -> [u8; 32] {
    blake2b_simd::Params::new().hash_length(32).personal(personal).hash(data).as_bytes().try_into().unwrap()
}

/// One signing hash per input, spending coins locked to `self_script`.
pub fn sighashes(chain: &str, self_script: &[u8], inputs: &[Input], outputs: &[Output], branch_id: u32, expiry_height: u32) -> Vec<[u8; 32]> {
    let outs = outputs_bytes(outputs);
    match chain {
        "bitcoin" | "litecoin" | "bitcoin-cash" => {
            let segwit = chain != "bitcoin-cash";
            let seq = if segwit { SEQ_SEGWIT } else { SEQ_FINAL };
            // scriptCode: P2PKH of the key hash, with its length.
            let pkh = if segwit { &self_script[2..22] } else { &self_script[3..23] };
            let mut script_code = vec![0x19, 0x76, 0xa9, 0x14];
            script_code.extend(pkh);
            script_code.extend([0x88, 0xac]);
            let (mut prevouts, mut seqs) = (vec![], vec![]);
            for i in inputs {
                outpoint(&mut prevouts, i);
                seqs.extend(seq.to_le_bytes());
            }
            let (hp, hs, ho) = (sha256d(&prevouts), sha256d(&seqs), sha256d(&outs));
            let hash_type: u32 = if segwit { 0x01 } else { 0x41 };
            inputs
                .iter()
                .map(|i| {
                    let mut pre = 2u32.to_le_bytes().to_vec();
                    pre.extend(hp);
                    pre.extend(hs);
                    outpoint(&mut pre, i);
                    pre.extend(&script_code);
                    pre.extend(i.value.to_le_bytes());
                    pre.extend(seq.to_le_bytes());
                    pre.extend(ho);
                    pre.extend(0u32.to_le_bytes());
                    pre.extend(hash_type.to_le_bytes());
                    sha256d(&pre)
                })
                .collect()
        }
        "zcash" => {
            let mut header = vec![];
            for v in [ZEC_V5_HEADER, ZEC_V5_GROUP, branch_id, 0, expiry_height] {
                header.extend(v.to_le_bytes());
            }
            let (mut prevouts, mut seqs, mut amounts, mut scripts) = (vec![], vec![], vec![], vec![]);
            for i in inputs {
                outpoint(&mut prevouts, i);
                seqs.extend(SEQ_FINAL.to_le_bytes());
                amounts.extend((i.value as i64).to_le_bytes());
                varint(&mut scripts, self_script.len());
                scripts.extend(self_script);
            }
            let hh = blake(b"ZTxIdHeadersHash", &header);
            let (hp, hs, ho) = (blake(b"ZTxIdPrevoutHash", &prevouts), blake(b"ZTxIdSequencHash", &seqs), blake(b"ZTxIdOutputsHash", &outs));
            let (ha, hsc) = (blake(b"ZTxTrAmountsHash", &amounts), blake(b"ZTxTrScriptsHash", &scripts));
            let (sapling, orchard) = (blake(b"ZTxIdSaplingHash", &[]), blake(b"ZTxIdOrchardHash", &[]));
            let mut personal = *b"ZcashTxHash_\0\0\0\0";
            personal[12..].copy_from_slice(&branch_id.to_le_bytes());
            inputs
                .iter()
                .map(|i| {
                    let mut txin = vec![];
                    outpoint(&mut txin, i);
                    txin.extend((i.value as i64).to_le_bytes());
                    varint(&mut txin, self_script.len());
                    txin.extend(self_script);
                    txin.extend(SEQ_FINAL.to_le_bytes());
                    let txin = blake(b"Zcash___TxInHash", &txin);
                    let t = blake(b"ZTxIdTranspaHash", &[&[1u8][..], &hp, &ha, &hsc, &hs, &ho, &txin].concat());
                    blake(&personal, &[&hh[..], &t, &sapling, &orchard].concat())
                })
                .collect()
        }
        _ => vec![],
    }
}
