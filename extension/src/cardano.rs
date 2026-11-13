//! Agent payments on Cardano (preprod): a Shelley-era transaction body in
//! CBOR (inputs, outputs, fee, TTL), signed by the account's ed25519 key
//! (path `cardano`) over BLAKE2b-256 of the body. Built here exactly as
//! `near/core/src/cardano.rs` builds the owner's sends.
//!
//! The fee is written in the body, and Cardano accepts a transaction only if
//! its inputs equal its outputs plus that fee. So an agent that misstates an
//! input's amount gets a transaction the chain rejects: what the extension
//! counts (payments plus fee) is what leaves the account.
//!
//! Addresses are raw Shelley address bytes, hex (what `addr_test1…` decodes
//! to): payment addresses (header types 0 to 7) on a test network (0).

/// The highest fee accepted, lovelace (a 2-in, 3-out payment costs about 0.2 ADA).
pub const MAX_FEE: u64 = 2_000_000;

pub struct Input {
    pub tx_hash: [u8; 32],
    pub index: u32,
}

fn head(out: &mut Vec<u8>, major: u8, n: u64) {
    let m = major << 5;
    match n {
        0..=23 => out.push(m | n as u8),
        24..=0xff => out.extend([m | 24, n as u8]),
        0x100..=0xffff => {
            out.push(m | 25);
            out.extend((n as u16).to_be_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(m | 26);
            out.extend((n as u32).to_be_bytes());
        }
        _ => {
            out.push(m | 27);
            out.extend(n.to_be_bytes());
        }
    }
}

fn bytes(out: &mut Vec<u8>, b: &[u8]) {
    head(out, 2, b.len() as u64);
    out.extend(b);
}

/// A Shelley payment address on a test network.
pub fn address_ok(b: &[u8]) -> bool {
    !b.is_empty() && b[0] >> 4 <= 7 && b[0] & 0x0f == 0 && (b.len() == 29 || b.len() == 57)
}

/// The body: {0: inputs, 1: outputs, 2: fee, 3: ttl}.
pub fn body(inputs: &[Input], outputs: &[(Vec<u8>, u64)], fee: u64, ttl: u64) -> Vec<u8> {
    let mut b = vec![];
    head(&mut b, 5, 4);
    head(&mut b, 0, 0);
    head(&mut b, 4, inputs.len() as u64);
    for i in inputs {
        head(&mut b, 4, 2);
        bytes(&mut b, &i.tx_hash);
        head(&mut b, 0, i.index as u64);
    }
    head(&mut b, 0, 1);
    head(&mut b, 4, outputs.len() as u64);
    for (a, v) in outputs {
        head(&mut b, 4, 2);
        bytes(&mut b, a);
        head(&mut b, 0, *v);
    }
    head(&mut b, 0, 2);
    head(&mut b, 0, fee);
    head(&mut b, 0, 3);
    head(&mut b, 0, ttl);
    b
}

/// What the key signs, and the transaction ID.
pub fn body_hash(body: &[u8]) -> [u8; 32] {
    blake2b_simd::Params::new().hash_length(32).hash(body).as_bytes().try_into().unwrap()
}
