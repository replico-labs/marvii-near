//! TON cells: building, hashing and bag-of-cells (BOC) encoding, for
//! ordinary (level 0) cells only. A copy of near/core/src/ton_cell.rs.
//!
//! A cell holds up to 1023 bits and up to 4 references. Its hash is
//! sha256(d1 || d2 || data padded with 1 then 0s || each ref's depth (u16 BE)
//! || each ref's hash), per the TVM whitepaper (3.1.4–3.1.5); checked in the
//! tests against @ton/core.

#![allow(dead_code)]

use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    pub bits: Vec<bool>,
    pub refs: Vec<Cell>,
}

/// An address in a workchain: 0 (basechain) or -1 (masterchain).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Addr {
    pub wc: i8,
    pub hash: [u8; 32],
}

impl Cell {
    pub fn new() -> Self {
        Cell::default()
    }
    fn check(&self) -> Result<(), &'static str> {
        if self.bits.len() > 1023 || self.refs.len() > 4 {
            return Err("cell overflow");
        }
        Ok(())
    }
    pub fn bit(&mut self, b: bool) -> &mut Self {
        self.bits.push(b);
        self
    }
    pub fn uint(&mut self, v: u128, n: usize) -> &mut Self {
        for i in (0..n).rev() {
            self.bits.push(i < 128 && (v >> i) & 1 == 1);
        }
        self
    }
    pub fn bytes(&mut self, b: &[u8]) -> &mut Self {
        for x in b {
            self.uint(*x as u128, 8);
        }
        self
    }
    /// Grams / VarUInteger 16: a 4-bit byte length, then the value.
    pub fn coins(&mut self, v: u128) -> &mut Self {
        let len = (128 - v.leading_zeros() as usize).div_ceil(8);
        self.uint(len as u128, 4);
        self.uint(v, len * 8)
    }
    /// MsgAddress: addr_none$00, or addr_std$10 with no anycast.
    pub fn addr(&mut self, a: Option<&Addr>) -> &mut Self {
        match a {
            None => self.uint(0, 2),
            Some(a) => {
                self.uint(0b100, 3);
                self.uint(a.wc as u8 as u128, 8);
                self.bytes(&a.hash)
            }
        }
    }
    pub fn push_ref(&mut self, c: Cell) -> &mut Self {
        self.refs.push(c);
        self
    }
    /// Maybe ^Cell.
    pub fn maybe_ref(&mut self, c: Option<Cell>) -> &mut Self {
        match c {
            None => self.bit(false),
            Some(c) => self.bit(true).push_ref(c),
        }
    }
    pub fn done(&mut self) -> Result<Cell, &'static str> {
        self.check()?;
        Ok(std::mem::take(self))
    }

    pub fn depth(&self) -> u16 {
        self.refs.iter().map(|r| r.depth() + 1).max().unwrap_or(0)
    }
    fn descriptors(&self) -> [u8; 2] {
        let n = self.bits.len();
        [self.refs.len() as u8, (n / 8 + n.div_ceil(8)) as u8]
    }
    fn data(&self) -> Vec<u8> {
        let mut bits = self.bits.clone();
        if bits.len() % 8 != 0 {
            bits.push(true);
            while bits.len() % 8 != 0 {
                bits.push(false);
            }
        }
        bits.chunks(8).map(|c| c.iter().fold(0u8, |a, b| a << 1 | *b as u8)).collect()
    }
    pub fn hash(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(self.descriptors());
        h.update(self.data());
        for r in &self.refs {
            h.update(r.depth().to_be_bytes());
        }
        for r in &self.refs {
            h.update(r.hash());
        }
        h.finalize().into()
    }

    /// One root, no index, no checksum; cells in pre-order (parents first).
    pub fn boc(&self) -> Vec<u8> {
        fn flatten<'a>(c: &'a Cell, out: &mut Vec<&'a Cell>) {
            out.push(c);
            for r in &c.refs {
                flatten(r, out);
            }
        }
        let mut list = vec![];
        flatten(self, &mut list);
        // Index of each cell: its position in pre-order.
        let mut idx = vec![];
        let mut next = 0usize;
        fn number(c: &Cell, next: &mut usize, idx: &mut Vec<Vec<usize>>) {
            *next += 1;
            let mut mine = vec![];
            idx.push(vec![]);
            let slot = idx.len() - 1;
            for r in &c.refs {
                mine.push(*next);
                number(r, next, idx);
            }
            idx[slot] = mine;
        }
        number(self, &mut next, &mut idx);
        let n = list.len();
        let size = if n < 256 { 1 } else if n < 65536 { 2 } else { 3 };
        let mut cells = vec![];
        for (c, refs) in list.iter().zip(&idx) {
            cells.extend(c.descriptors());
            cells.extend(c.data());
            for r in refs {
                cells.extend(&(*r as u64).to_be_bytes()[8 - size..]);
            }
        }
        let off = (64 - (cells.len() as u64).leading_zeros() as usize).div_ceil(8).max(1);
        let mut out = vec![0xb5, 0xee, 0x9c, 0x72, size as u8, off as u8];
        out.extend(&(n as u64).to_be_bytes()[8 - size..]);
        out.extend(&1u64.to_be_bytes()[8 - size..]);
        out.extend(&0u64.to_be_bytes()[8 - size..]);
        out.extend(&(cells.len() as u64).to_be_bytes()[8 - off..]);
        out.extend(&0u64.to_be_bytes()[8 - size..]);
        out.extend(cells);
        out
    }

    /// Reads a one-root BOC of ordinary cells (index and checksum allowed,
    /// ignored).
    pub fn from_boc(b: &[u8]) -> Result<Cell, &'static str> {
        let bad = "bad BOC";
        if b.len() < 6 || b[..4] != [0xb5, 0xee, 0x9c, 0x72] {
            return Err(bad);
        }
        let has_idx = b[4] & 0x80 != 0;
        let size = (b[4] & 7) as usize;
        let off = b[5] as usize;
        if size == 0 || size > 4 || off == 0 || off > 8 {
            return Err(bad);
        }
        let mut p = 6;
        let read = |n: usize, p: &mut usize| -> Result<usize, &'static str> {
            let s = b.get(*p..*p + n).ok_or(bad)?;
            *p += n;
            Ok(s.iter().fold(0usize, |a, x| a << 8 | *x as usize))
        };
        let cells = read(size, &mut p)?;
        let roots = read(size, &mut p)?;
        let _absent = read(size, &mut p)?;
        let _total = read(off, &mut p)?;
        if roots != 1 || cells == 0 || cells > 4096 {
            return Err(bad);
        }
        let root = read(size, &mut p)?;
        if has_idx {
            p += cells * off;
        }
        let mut raw: Vec<(Vec<bool>, Vec<usize>)> = vec![];
        for _ in 0..cells {
            let d = b.get(p..p + 2).ok_or(bad)?;
            p += 2;
            let (nrefs, d2) = ((d[0] & 7) as usize, d[1] as usize);
            if d[0] & 0xf8 != 0 || nrefs > 4 {
                return Err("only ordinary cells");
            }
            let len = d2.div_ceil(2);
            let data = b.get(p..p + len).ok_or(bad)?;
            p += len;
            let mut bits: Vec<bool> = data.iter().flat_map(|x| (0..8).rev().map(move |i| x >> i & 1 == 1)).collect();
            if d2 % 2 == 1 {
                // Strip the completion tag: the last 1 and the 0s after it.
                while bits.last() == Some(&false) {
                    bits.pop();
                }
                if bits.pop() != Some(true) {
                    return Err(bad);
                }
            }
            let mut refs = vec![];
            for _ in 0..nrefs {
                let s = b.get(p..p + size).ok_or(bad)?;
                p += size;
                refs.push(s.iter().fold(0usize, |a, x| a << 8 | *x as usize));
            }
            raw.push((bits, refs));
        }
        // References point forward; build from the last cell back.
        let mut built: Vec<Option<Cell>> = vec![None; cells];
        for i in (0..cells).rev() {
            let (bits, refs) = &raw[i];
            let mut c = Cell { bits: bits.clone(), refs: vec![] };
            for r in refs {
                if *r <= i || *r >= cells {
                    return Err(bad);
                }
                c.refs.push(built[*r].clone().ok_or(bad)?);
            }
            c.check()?;
            built[i] = Some(c);
        }
        built.get(root).cloned().flatten().ok_or(bad)
    }
}

pub fn crc16(b: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &x in b {
        crc ^= (x as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { crc << 1 ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

const B64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// A user-friendly address: base64url of tag, workchain, hash, crc16.
/// Tag 0x11 bounceable or 0x51 not, plus 0x80 on testnet.
pub fn friendly(a: &Addr, bounceable: bool, testnet: bool) -> String {
    let mut b = vec![if bounceable { 0x11u8 } else { 0x51 } | if testnet { 0x80 } else { 0 }, a.wc as u8];
    b.extend(a.hash);
    b.extend(crc16(&b).to_be_bytes());
    b.chunks(3)
        .flat_map(|c| {
            let n = (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32;
            (0..4).rev().map(move |i| B64URL[(n >> (6 * i) & 63) as usize] as char)
        })
        .collect()
}

/// Parses `0:<hex>` or a user-friendly address (either base64 alphabet).
/// Returns the address and whether it asks for bounceable messages.
pub fn parse_address(s: &str) -> Result<(Addr, bool), &'static str> {
    let bad = "not a TON address";
    if let Some((wc, h)) = s.split_once(':') {
        let wc: i8 = wc.parse().map_err(|_| bad)?;
        let hash: [u8; 32] = hex::decode(h).map_err(|_| bad)?.try_into().map_err(|_| bad)?;
        if wc != 0 && wc != -1 {
            return Err(bad);
        }
        return Ok((Addr { wc, hash }, true));
    }
    if s.len() != 48 {
        return Err(bad);
    }
    let mut bits = 0u32;
    let mut b = vec![];
    for (i, ch) in s.bytes().enumerate() {
        let v = match ch {
            b'A'..=b'Z' => ch - b'A',
            b'a'..=b'z' => ch - b'a' + 26,
            b'0'..=b'9' => ch - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return Err(bad),
        } as u32;
        bits = bits << 6 | v;
        if i % 4 == 3 {
            b.extend([(bits >> 16) as u8, (bits >> 8) as u8, bits as u8]);
            bits = 0;
        }
    }
    if crc16(&b[..34]).to_be_bytes() != b[34..36] {
        return Err(bad);
    }
    let tag = b[0] & 0x7f;
    if tag != 0x11 && tag != 0x51 {
        return Err(bad);
    }
    let wc = b[1] as i8;
    if wc != 0 && wc != -1 {
        return Err(bad);
    }
    Ok((Addr { wc, hash: b[2..34].try_into().unwrap() }, tag == 0x11))
}

/// An internal message (CommonMsgInfoRelaxed int_msg_info$0) with no state
/// init; the body, when there is one, goes in a reference.
pub fn internal_message(to: &Addr, value: u128, bounce: bool, body: Option<Cell>) -> Result<Cell, &'static str> {
    let mut c = Cell::new();
    c.bit(false).bit(true).bit(bounce).bit(false); // int_msg_info, ihr_disabled, bounce, bounced
    c.addr(None).addr(Some(to)).coins(value).bit(false); // src, dest, value, no extra currencies
    c.coins(0).coins(0).uint(0, 64).uint(0, 32); // ihr_fee, fwd_fee, created_lt, created_at
    c.bit(false); // no init
    match body {
        None => c.bit(false),
        Some(b) => c.bit(true).push_ref(b),
    };
    c.done()
}

/// A text comment body: op 0, then the text (one cell, at most 123 bytes).
pub fn comment(text: &str) -> Result<Cell, &'static str> {
    if text.len() > 123 {
        return Err("comment too long");
    }
    Cell::new().uint(0, 32).bytes(text.as_bytes()).done()
}

/// Wallet v4r2's subwallet id on workchain 0.
pub const V4_SUBWALLET: u32 = 698983191;
/// Send mode 3: pay transfer fees separately and ignore errors. The only
/// mode used: it can't send the whole balance (128) or destroy the wallet (32).
pub const SEND_MODE: u8 = 3;

/// The part of a wallet v4r2 external message its key signs: subwallet id,
/// valid-until, seqno, op 0 (send), then a mode byte and a reference for
/// each of up to four internal messages. The wallet checks the signature
/// over this cell's hash.
pub fn v4_signing_message(seqno: u32, valid_until: u32, messages: Vec<Cell>) -> Result<Cell, &'static str> {
    if messages.is_empty() || messages.len() > 4 {
        return Err("1 to 4 messages");
    }
    let mut c = Cell::new();
    c.uint(V4_SUBWALLET as u128, 32).uint(valid_until as u128, 32).uint(seqno as u128, 32).uint(0, 8);
    for m in messages {
        c.uint(SEND_MODE as u128, 8).push_ref(m);
    }
    c.done()
}
