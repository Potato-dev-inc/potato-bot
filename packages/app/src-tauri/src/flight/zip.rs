//! A small ZIP writer (deflate, no ZIP64, UTF-8 names) so the diagnostic file needs no new crate:
//! `flate2` (MIT OR Apache-2.0) does the compression and `crc32fast` (MIT OR Apache-2.0) the checksums,
//! and both were already in the build through `ureq`.
//!
//! Format: PKWARE APPNOTE 6.3.x. Local header + data per file, then the central directory and the
//! end-of-central-directory record.

/// Read a zip made by [`ZipWriter`] back: `(name, bytes)` per file. Used by tests to prove the
/// archive is well formed (checksums included); not a general reader.
#[cfg(test)]
pub fn read_back(zip: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    use flate2::read::DeflateDecoder;
    use std::io::Read;
    let u16_at = |at: usize| u16::from_le_bytes([zip[at], zip[at + 1]]) as usize;
    let u32_at = |at: usize| u32::from_le_bytes([zip[at], zip[at + 1], zip[at + 2], zip[at + 3]]);
    let eocd = zip.len().checked_sub(22).ok_or("too short")?;
    if u32_at(eocd) != 0x0605_4b50 {
        return Err("no end-of-central-directory record".into());
    }
    let count = u16_at(eocd + 10);
    let mut at = u32_at(eocd + 16) as usize;
    let mut out = Vec::new();
    for _ in 0..count {
        if u32_at(at) != 0x0201_4b50 {
            return Err("bad central directory entry".into());
        }
        let crc = u32_at(at + 16);
        let compressed = u32_at(at + 20) as usize;
        let size = u32_at(at + 24) as usize;
        let name_len = u16_at(at + 28);
        let extra = u16_at(at + 30) + u16_at(at + 32);
        let local = u32_at(at + 42) as usize;
        let name = String::from_utf8(zip[at + 46..at + 46 + name_len].to_vec()).map_err(|e| e.to_string())?;
        if u32_at(local) != 0x0403_4b50 {
            return Err("bad local header".into());
        }
        let data_at = local + 30 + u16_at(local + 26) + u16_at(local + 28);
        let mut data = Vec::with_capacity(size);
        DeflateDecoder::new(&zip[data_at..data_at + compressed]).read_to_end(&mut data).map_err(|e| e.to_string())?;
        if data.len() != size || crc32fast::hash(&data) != crc {
            return Err(format!("{name}: size or checksum mismatch"));
        }
        out.push((name, data));
        at += 46 + name_len + extra;
    }
    Ok(out)
}
