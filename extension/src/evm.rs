//! EVM transactions for Chain Signatures: the extension builds the EIP-1559
//! signing hash itself from typed fields, so what gets signed is exactly what
//! the rules checked. It never signs a hash it was handed.

use near_sdk::env;

/// `transfer(address,uint256)`, computed with `cast sig`.
pub const SEL_TRANSFER: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb];
/// `approve(address,uint256)`, computed with `cast sig`.
pub const SEL_APPROVE: [u8; 4] = [0x09, 0x5e, 0xa7, 0xb3];

/// An unsigned EIP-1559 (type 2) transaction with an empty access list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Eip1559 {
    pub chain_id: u64,
    pub nonce: u64,
    pub max_priority_fee_per_gas: u128,
    pub max_fee_per_gas: u128,
    pub gas_limit: u64,
    pub to: [u8; 20],
    pub value: u128,
    pub data: Vec<u8>,
}

impl Eip1559 {
    /// keccak256(0x02 || rlp([chainId, nonce, maxPriorityFeePerGas,
    /// maxFeePerGas, gasLimit, to, value, data, accessList])).
    /// The chain id is inside the signed data (EIP-155 style), so the
    /// signature can't be replayed on another chain.
    pub fn signing_hash(&self) -> [u8; 32] {
        let mut out = vec![0x02];
        out.extend(self.rlp_payload());
        env::keccak256_array(&out)
    }

    pub fn rlp_payload(&self) -> Vec<u8> {
        let mut items = Vec::new();
        rlp_uint(&mut items, self.chain_id as u128);
        rlp_uint(&mut items, self.nonce as u128);
        rlp_uint(&mut items, self.max_priority_fee_per_gas);
        rlp_uint(&mut items, self.max_fee_per_gas);
        rlp_uint(&mut items, self.gas_limit as u128);
        rlp_bytes(&mut items, &self.to);
        rlp_uint(&mut items, self.value);
        rlp_bytes(&mut items, &self.data);
        items.push(0xc0); // empty access list
        let mut out = Vec::new();
        rlp_len_prefix(&mut out, items.len(), 0xc0);
        out.extend(items);
        out
    }
}

fn rlp_uint(out: &mut Vec<u8>, v: u128) {
    let be = v.to_be_bytes();
    let first = be.iter().position(|b| *b != 0).unwrap_or(be.len());
    rlp_bytes(out, &be[first..]);
}

fn rlp_bytes(out: &mut Vec<u8>, b: &[u8]) {
    if b.len() == 1 && b[0] < 0x80 {
        out.push(b[0]);
    } else {
        rlp_len_prefix(out, b.len(), 0x80);
        out.extend_from_slice(b);
    }
}

fn rlp_len_prefix(out: &mut Vec<u8>, len: usize, base: u8) {
    if len <= 55 {
        out.push(base + len as u8);
    } else {
        let be = (len as u64).to_be_bytes();
        let first = be.iter().position(|b| *b != 0).unwrap_or(be.len());
        out.push(base + 55 + (be.len() - first) as u8);
        out.extend_from_slice(&be[first..]);
    }
}

/// What a call does, decoded from its data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decoded {
    /// No data: a plain transfer of the native coin.
    Native,
    /// ERC-20 `transfer(to, amount)` on the token at the transaction's `to`.
    Transfer { recipient: [u8; 20], amount: u128 },
    /// ERC-20 `approve(spender, amount)` on the token at the transaction's `to`.
    Approve { spender: [u8; 20], amount: u128 },
    /// Any other function, identified by its selector.
    Call { selector: [u8; 4] },
}

pub fn decode(data: &[u8]) -> Result<Decoded, &'static str> {
    if data.is_empty() {
        return Ok(Decoded::Native);
    }
    if data.len() < 4 {
        return Err("call data shorter than a selector");
    }
    let selector: [u8; 4] = data[..4].try_into().unwrap();
    if selector == SEL_TRANSFER || selector == SEL_APPROVE {
        // Exactly two 32-byte words: no trailing bytes the token might read.
        if data.len() != 4 + 64 {
            return Err("transfer/approve data has the wrong length");
        }
        let addr = word_address(&data[4..36])?;
        let amount = word_u128(&data[36..68])?;
        return Ok(if selector == SEL_TRANSFER {
            Decoded::Transfer { recipient: addr, amount }
        } else {
            Decoded::Approve { spender: addr, amount }
        });
    }
    Ok(Decoded::Call { selector })
}

fn word_address(w: &[u8]) -> Result<[u8; 20], &'static str> {
    if w[..12].iter().any(|b| *b != 0) {
        return Err("address word has dirty high bytes");
    }
    Ok(w[12..32].try_into().unwrap())
}

/// Amounts above u128 are refused: no cap could cover them.
fn word_u128(w: &[u8]) -> Result<u128, &'static str> {
    if w[..16].iter().any(|b| *b != 0) {
        return Err("amount too large");
    }
    Ok(u128::from_be_bytes(w[16..32].try_into().unwrap()))
}

pub fn parse_address(s: &str) -> Result<[u8; 20], &'static str> {
    let s = s.strip_prefix("0x").ok_or("address must start with 0x")?;
    let v = hex::decode(s).map_err(|_| "address is not hex")?;
    v.try_into().map_err(|_| "address must be 20 bytes")
}

pub fn parse_hex(s: &str) -> Result<Vec<u8>, &'static str> {
    let s = s.strip_prefix("0x").ok_or("hex must start with 0x")?;
    hex::decode(s).map_err(|_| "not hex")
}

pub fn fmt_address(a: &[u8; 20]) -> String {
    format!("0x{}", hex::encode(a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rlp_matches_known_encodings() {
        let mut o = Vec::new();
        rlp_uint(&mut o, 0);
        assert_eq!(o, [0x80]);
        let mut o = Vec::new();
        rlp_uint(&mut o, 0x7f);
        assert_eq!(o, [0x7f]);
        let mut o = Vec::new();
        rlp_uint(&mut o, 0x400);
        assert_eq!(o, [0x82, 0x04, 0x00]);
        let mut o = Vec::new();
        rlp_bytes(&mut o, &[0u8; 56]);
        assert_eq!(&o[..2], &[0xb8, 56]);
    }

    #[test]
    fn decodes_erc20_and_refuses_malformed() {
        let mut d = SEL_APPROVE.to_vec();
        d.extend([0u8; 12]);
        d.extend([0x11u8; 20]);
        d.extend([0u8; 31]);
        d.push(5);
        assert_eq!(decode(&d), Ok(Decoded::Approve { spender: [0x11; 20], amount: 5 }));
        let mut long = d.clone();
        long.push(0);
        assert!(decode(&long).is_err());
        let mut dirty = d.clone();
        dirty[4] = 1;
        assert!(decode(&dirty).is_err());
        let mut huge = d.clone();
        huge[36] = 1;
        assert!(decode(&huge).is_err());
        assert!(decode(&[1, 2]).is_err());
        assert_eq!(decode(&[]), Ok(Decoded::Native));
    }
}
