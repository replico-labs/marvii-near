//! EVM transactions for Chain Signatures: the extension builds the EIP-1559
//! signing hash itself from typed fields, so what gets signed is exactly what
//! the rules checked. It never signs a hash it was handed.

use near_sdk::env;

/// `transfer(address,uint256)`, computed with `cast sig`.
pub const SEL_TRANSFER: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb];
/// `approve(address,uint256)`, computed with `cast sig`.
pub const SEL_APPROVE: [u8; 4] = [0x09, 0x5e, 0xa7, 0xb3];

/// Venue calls whose layout the extension knows, so it can check where the
/// result goes. Selectors computed with keccak256 of the signatures.
/// Aave V3 Pool `supply(address,uint256,address,uint16)`: onBehalfOf.
pub const SEL_AAVE_SUPPLY: [u8; 4] = [0x61, 0x7b, 0xa0, 0x37];
/// Aave V3 Pool `withdraw(address,uint256,address)`: to.
pub const SEL_AAVE_WITHDRAW: [u8; 4] = [0x69, 0x32, 0x8d, 0xec];
/// Aave V3 Pool `repay(address,uint256,uint256,address)`: onBehalfOf.
pub const SEL_AAVE_REPAY: [u8; 4] = [0x57, 0x3a, 0xde, 0x81];
/// Aave V3 Pool `borrow(address,uint256,uint256,uint16,address)`:
/// onBehalfOf. The borrowed tokens go to the caller, the account itself.
pub const SEL_AAVE_BORROW: [u8; 4] = [0xa4, 0x15, 0xbc, 0xad];
/// Uniswap V3 SwapRouter02 `exactInputSingle((address,address,uint24,
/// address,uint256,uint256,uint160))`: recipient.
pub const SEL_UNI_EXACT_INPUT_SINGLE: [u8; 4] = [0x04, 0xe4, 0x5a, 0xaf];

/// Lido (stETH, wstETH and the withdrawal queue; `cast sig`). What they
/// produce goes to the caller, the account itself, except a withdrawal
/// request's NFT, which goes to its `owner` argument.
/// stETH `submit(address)`: ETH in (the value), stETH to the caller.
pub const SEL_LIDO_SUBMIT: [u8; 4] = [0xa1, 0x90, 0x3e, 0xab];
/// wstETH `wrap(uint256)` and `unwrap(uint256)`.
pub const SEL_WSTETH_WRAP: [u8; 4] = [0xea, 0x59, 0x8c, 0xb0];
pub const SEL_WSTETH_UNWRAP: [u8; 4] = [0xde, 0x0e, 0x9a, 0x3e];
/// WithdrawalQueueERC721 `requestWithdrawals(uint256[],address)` and
/// `requestWithdrawalsWstETH(uint256[],address)`: owner.
pub const SEL_LIDO_REQUEST: [u8; 4] = [0xd6, 0x68, 0x10, 0x42];
pub const SEL_LIDO_REQUEST_WSTETH: [u8; 4] = [0x19, 0xaa, 0x62, 0x57];
/// `claimWithdrawal(uint256)`: the ETH goes to the caller.
pub const SEL_LIDO_CLAIM: [u8; 4] = [0xf8, 0x44, 0x44, 0x36];

/// ERC-721 `setApprovalForAll(address,bool)`.
pub const SEL_SET_APPROVAL_FOR_ALL: [u8; 4] = [0xa2, 0x2c, 0xb4, 0x65];
/// Seaport `fulfillBasicOrder(BasicOrderParameters)` and its gas-efficient
/// twin `fulfillBasicOrder_efficient_6GL6yc` (same ABI): the NFT goes to the
/// caller. Only the routes where the caller pays (ETH or ERC-20 for an
/// ERC-721 or ERC-1155); the others would hand the account's NFT over for
/// whatever the order asks.
pub const SEL_SEAPORT_BASIC: [u8; 4] = [0xfb, 0x0f, 0x3e, 0xe1];
pub const SEL_SEAPORT_BASIC_EFFICIENT: [u8; 4] = [0x00, 0x00, 0x00, 0x00];

/// For a call whose layout is known, the address its result goes to (who
/// gets the deposit, the withdrawn tokens or the swap's output), which must
/// be the account's own. `None` for any other call. The data must have
/// exactly the expected length, so nothing extra can ride along.
pub fn beneficiary(data: &[u8]) -> Result<Option<[u8; 20]>, &'static str> {
    if data.len() < 4 {
        return Ok(None);
    }
    let sel: [u8; 4] = data[..4].try_into().unwrap();
    match sel {
        SEL_LIDO_SUBMIT => {
            if data.len() != 36 {
                return Err("venue call data has the wrong length");
            }
            word_address(&data[4..36])?; // the referral: anyone, it gets nothing
            return Ok(None);
        }
        SEL_SEAPORT_BASIC | SEL_SEAPORT_BASIC_EFFICIENT => {
            // One tuple: its offset (0x20), then 18 head words; basicOrderType is the 9th.
            if data.len() < 4 + 32 * 19 || word_u128(&data[4..36])? != 0x20 {
                return Err("venue call data has the wrong length");
            }
            let order_type = word_u128(&data[4 + 32 * 9..4 + 32 * 10])?;
            if order_type > 23 || order_type / 4 > 3 {
                return Err("only buying an NFT through Seaport");
            }
            return Ok(None);
        }
        SEL_WSTETH_WRAP | SEL_WSTETH_UNWRAP | SEL_LIDO_CLAIM => {
            if data.len() != 36 {
                return Err("venue call data has the wrong length");
            }
            return Ok(None);
        }
        // (offset 0x40, owner, n, n amounts): strictly this encoding, so the
        // owner word is the one the contract reads.
        SEL_LIDO_REQUEST | SEL_LIDO_REQUEST_WSTETH => {
            let w = |i: usize| data.get(4 + 32 * i..4 + 32 * (i + 1)).ok_or("venue call data has the wrong length");
            let n = word_u128(w(2)?)? as usize;
            if word_u128(w(0)?)? != 0x40 || n == 0 || n > 16 || data.len() != 4 + 32 * (3 + n) {
                return Err("venue call data has the wrong length");
            }
            return Ok(Some(word_address(w(1)?)?));
        }
        _ => {}
    }
    let (words, at) = match sel {
        SEL_AAVE_SUPPLY => (4, 2),
        SEL_AAVE_WITHDRAW => (3, 2),
        SEL_AAVE_REPAY => (4, 3),
        SEL_AAVE_BORROW => (5, 4),
        SEL_UNI_EXACT_INPUT_SINGLE => (7, 3),
        _ => return Ok(None),
    };
    if data.len() != 4 + 32 * words {
        return Err("venue call data has the wrong length");
    }
    // Every address word in these layouts must be clean.
    let addr_words: &[usize] = match sel {
        SEL_AAVE_SUPPLY => &[0, 2],
        SEL_AAVE_WITHDRAW => &[0, 2],
        SEL_AAVE_REPAY => &[0, 3],
        SEL_AAVE_BORROW => &[0, 4],
        _ => &[0, 1, 3],
    };
    for w in addr_words {
        word_address(&data[4 + 32 * w..4 + 32 * (w + 1)])?;
    }
    Ok(Some(word_address(&data[4 + 32 * at..4 + 32 * (at + 1)])?))
}

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

pub fn word_address(w: &[u8]) -> Result<[u8; 20], &'static str> {
    if w[..12].iter().any(|b| *b != 0) {
        return Err("address word has dirty high bytes");
    }
    Ok(w[12..32].try_into().unwrap())
}

/// Amounts above u128 are refused: no cap could cover them.
pub fn word_u128(w: &[u8]) -> Result<u128, &'static str> {
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

/// For an Aave borrow: (asset, amount). Only the variable rate (mode 2).
pub fn aave_borrow(data: &[u8]) -> Result<Option<([u8; 20], u128)>, &'static str> {
    if data.len() != 4 + 32 * 5 || data[..4] != SEL_AAVE_BORROW {
        return Ok(None);
    }
    let w = |i: usize| &data[4 + 32 * i..4 + 32 * (i + 1)];
    if w(2)[..31].iter().any(|b| *b != 0) || w(2)[31] != 2 {
        return Err("borrow at the variable rate only");
    }
    if w(1)[..16].iter().any(|b| *b != 0) {
        return Err("amount too large");
    }
    Ok(Some((word_address(w(0))?, u128::from_be_bytes(w(1)[16..].try_into().unwrap()))))
}
