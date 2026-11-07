//! Byte format used by every Spaces structure. Integers are big-endian.
//! str = u16 length + UTF-8. blob = u32 length + bytes. Nothing else.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireError(pub &'static str);

#[derive(Default)]
pub struct Writer { buf: Vec<u8> }

impl Writer {
    pub fn new() -> Self { Self { buf: Vec::new() } }
    pub fn u8(&mut self, v: u8) -> &mut Self { self.buf.push(v); self }
    pub fn u16(&mut self, v: u16) -> &mut Self { self.buf.extend_from_slice(&v.to_be_bytes()); self }
    pub fn u32(&mut self, v: u32) -> &mut Self { self.buf.extend_from_slice(&v.to_be_bytes()); self }
    pub fn u64(&mut self, v: u64) -> &mut Self { self.buf.extend_from_slice(&v.to_be_bytes()); self }
    pub fn fixed(&mut self, b: &[u8]) -> &mut Self { self.buf.extend_from_slice(b); self }
    /// Panics if longer than 65535 bytes; callers clean and cap text first.
    pub fn str(&mut self, s: &str) -> &mut Self {
        assert!(s.len() <= u16::MAX as usize, "str too long");
        self.u16(s.len() as u16).fixed(s.as_bytes())
    }
    pub fn blob(&mut self, b: &[u8]) -> &mut Self {
        assert!(b.len() <= u32::MAX as usize, "blob too long");
        self.u32(b.len() as u32).fixed(b)
    }
    pub fn len(&self) -> usize { self.buf.len() }
    pub fn bytes(&self) -> &[u8] { &self.buf }
    pub fn finish(self) -> Vec<u8> { self.buf }
}

pub struct Reader<'a> { buf: &'a [u8], pos: usize }

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self { Self { buf, pos: 0 } }
    pub fn pos(&self) -> usize { self.pos }
    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let end = self.pos.checked_add(n).ok_or(WireError("overflow"))?;
        if end > self.buf.len() { return Err(WireError("short")); }
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    pub fn u8(&mut self) -> Result<u8, WireError> { Ok(self.take(1)?[0]) }
    pub fn u16(&mut self) -> Result<u16, WireError> { Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap())) }
    pub fn u32(&mut self) -> Result<u32, WireError> { Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap())) }
    pub fn u64(&mut self) -> Result<u64, WireError> { Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap())) }
    pub fn fixed<const N: usize>(&mut self) -> Result<[u8; N], WireError> { Ok(self.take(N)?.try_into().unwrap()) }
    pub fn str(&mut self, max_bytes: usize) -> Result<String, WireError> {
        let n = self.u16()? as usize;
        if n > max_bytes { return Err(WireError("str too long")); }
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| WireError("not utf-8"))
    }
    pub fn blob(&mut self, max: usize) -> Result<&'a [u8], WireError> {
        let n = self.u32()? as usize;
        if n > max { return Err(WireError("blob too long")); }
        self.take(n)
    }
    /// Unread bytes. A reader of an older format uses this to skip fields added by a newer one.
    pub fn remaining(&self) -> usize { self.buf.len() - self.pos }
    /// Takes every unread byte (unknown trailing fields) so `done()` then succeeds.
    pub fn skip_rest(&mut self) -> &'a [u8] {
        let s = &self.buf[self.pos..];
        self.pos = self.buf.len();
        s
    }
    pub fn done(&self) -> Result<(), WireError> {
        if self.pos == self.buf.len() { Ok(()) } else { Err(WireError("trailing bytes")) }
    }
}

const B32: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// RFC 4648 base32, lowercase, no padding.
pub fn base32_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() * 8 + 4) / 5);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &b in data {
        acc = (acc << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(B32[((acc >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 { out.push(B32[((acc << (5 - bits)) & 31) as usize] as char); }
    out
}

/// Accepts upper or lower case; rejects any other character and non-zero leftover bits.
pub fn base32_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 5 / 8);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let v = B32.iter().position(|&x| x == c.to_ascii_lowercase())? as u32;
        acc = (acc << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    if bits >= 5 || (acc & ((1 << bits) - 1)) != 0 { return None; }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn writer_reader_round_trip() {
        let mut w = Writer::new();
        w.u8(1).u16(2).u32(3).u64(4).fixed(&[5u8; 4]).str("héllo").blob(&[6, 7]);
        let b = w.finish();
        let mut r = Reader::new(&b);
        assert_eq!(r.u8().unwrap(), 1);
        assert_eq!(r.u16().unwrap(), 2);
        assert_eq!(r.u32().unwrap(), 3);
        assert_eq!(r.u64().unwrap(), 4);
        assert_eq!(r.fixed::<4>().unwrap(), [5u8; 4]);
        assert_eq!(r.str(100).unwrap(), "héllo");
        assert_eq!(r.blob(10).unwrap(), &[6, 7]);
        assert!(r.done().is_ok());
    }
    #[test]
    fn short_input_is_an_error_not_a_panic() {
        assert_eq!(Reader::new(&[0, 1]).u32(), Err(WireError("short")));
    }
    #[test]
    fn too_long_str_is_rejected() {
        let mut w = Writer::new();
        w.str("abcdef");
        assert_eq!(Reader::new(&w.finish()).str(3), Err(WireError("str too long")));
    }
    #[test]
    fn trailing_bytes_are_rejected() {
        let mut r = Reader::new(&[1, 2]);
        r.u8().unwrap();
        assert_eq!(r.done(), Err(WireError("trailing bytes")));
    }
    #[test]
    fn unknown_trailing_fields_can_be_skipped() {
        let mut r = Reader::new(&[1, 9, 9, 9]);
        r.u8().unwrap();
        assert_eq!(r.remaining(), 3);
        assert_eq!(r.skip_rest(), &[9, 9, 9]);
        assert!(r.done().is_ok());
    }
    #[test]
    fn base32_matches_rfc4648_vectors() {
        assert_eq!(base32_encode(b""), "");
        assert_eq!(base32_encode(b"f"), "my");
        assert_eq!(base32_encode(b"fo"), "mzxq");
        assert_eq!(base32_encode(b"foo"), "mzxw6");
        assert_eq!(base32_encode(b"foob"), "mzxw6yq");
        assert_eq!(base32_encode(b"fooba"), "mzxw6ytb");
        assert_eq!(base32_encode(b"foobar"), "mzxw6ytboi");
    }
    #[test]
    fn base32_round_trip_and_rejects_junk() {
        let data: Vec<u8> = (0u8..=200).collect();
        assert_eq!(base32_decode(&base32_encode(&data)).unwrap(), data);
        assert_eq!(base32_decode("MZXW6YTBOI").unwrap(), b"foobar");
        assert_eq!(base32_decode("mzxw6ytb0i"), None); // '0' is not in the alphabet
        assert_eq!(base32_decode("my"), Some(b"f".to_vec()));
        assert_eq!(base32_decode("mz"), None); // leftover non-zero bits
        assert_eq!(base32_decode("mz7"), None); // leftover non-zero bits
    }
}
