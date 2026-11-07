//! Keep a space small: 90 days or 200 MB, whichever comes first (founder decision 13).
//! Control ops are never touched: the roster must always replay from genesis.
use rusqlite::params;

use super::ids::DeviceKey;
use super::limits::{AUTHOR_KEEP_BYTES, KEEP_BYTES, KEEP_MS, TRIM_TO_BYTES};
use super::store::SpaceStore;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct PruneReport {
    pub old_entries: u64,
    pub size_entries: u64,
    pub files: u64,
    pub bytes_after: u64,
}

pub fn used_bytes(s: &SpaceStore) -> rusqlite::Result<u64> {
    let e: i64 = s.conn.query_row("SELECT COALESCE(SUM(size), 0) FROM entries", [], |r| r.get(0))?;
    let f: i64 = s.conn.query_row("SELECT COALESCE(SUM(size), 0) FROM files WHERE have = 1", [], |r| r.get(0))?;
    Ok((e + f) as u64)
}

/// `delete_file` removes a cached file from disk (the service passes a closure; tests pass a recorder).
/// Steps 1 to 4 run in one transaction; the incremental vacuum runs after it commits.
pub fn prune(s: &SpaceStore, now_ms: u64, delete_file: &dyn Fn(&[u8; 32])) -> rusqlite::Result<PruneReport> {
    let mut report = PruneReport::default();
    s.in_tx(|s| {
        report.old_entries = prune_by_age(s, now_ms)?;
        report.files = prune_files(s, delete_file)?;
        report.size_entries = prune_by_size(s)?;
        s.conn.execute("DELETE FROM reactions WHERE target NOT IN (SELECT id FROM entries)", [])?;
        s.conn.execute("DELETE FROM files WHERE offer NOT IN (SELECT id FROM entries) AND have = 0", [])?;
        Ok(())
    })?;
    s.conn.execute_batch("PRAGMA incremental_vacuum(2000);")?;
    report.bytes_after = used_bytes(s)?;
    Ok(report)
}

/// 1. Anything older than KEEP_MS goes, and each author's floor rises to the newest seq that went, so sync never
/// asks for it again. "Gone" placeholders (created_ms 0) stay: they cost nothing and keep deleted seqs answered.
fn prune_by_age(s: &SpaceStore, now_ms: u64) -> rusqlite::Result<u64> {
    let cutoff = now_ms.saturating_sub(KEEP_MS) as i64;
    s.conn.execute(
        "UPDATE authors SET floor = MAX(floor, COALESCE((SELECT MAX(seq) FROM entries \
         WHERE device = authors.device AND created_ms > 0 AND created_ms < ?1), 0))",
        params![cutoff],
    )?;
    let n = s.conn.execute("DELETE FROM entries WHERE created_ms > 0 AND created_ms < ?1", params![cutoff])?;
    Ok(n as u64)
}

/// 2. Over the limit: cached files go first, oldest first, until the space is back under the trim size.
fn prune_files(s: &SpaceStore, delete_file: &dyn Fn(&[u8; 32])) -> rusqlite::Result<u64> {
    if used_bytes(s)? <= KEEP_BYTES { return Ok(0); }
    let shas: Vec<Vec<u8>> = {
        let mut st = s.conn.prepare("SELECT sha FROM files WHERE have = 1 ORDER BY cached_ms ASC, sha")?;
        let rows = st.query_map([], |r| r.get::<_, Vec<u8>>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut n = 0;
    for sha in shas {
        if used_bytes(s)? <= TRIM_TO_BYTES { break; }
        if let Ok(arr) = <[u8; 32]>::try_from(sha.as_slice()) { delete_file(&arr); }
        s.conn.execute("UPDATE files SET have = 0, cached_ms = NULL WHERE sha = ?1", params![sha])?;
        n += 1;
    }
    Ok(n)
}

/// Kinds that are never evicted: deletion tombstones (Delete) and Leave are tiny and other entries depend on them.
const KEEP_KINDS: &str = "(4, 5)";

/// 3. Only when the space is over the limit: per-author quota, then biggest authors first.
/// Every author is held to a byte cap `L`: at most `AUTHOR_KEEP_BYTES`, and when the space is still over the limit,
/// the lowest `L` that brings the entries back under the trim size (so the biggest authors are cut first and
/// everyone under `L` loses nothing). An author's entries go in their own seq order, oldest first, so the claimed
/// `created_ms` buys nothing and the floor stays exact. Tombstones, Leave and "gone" placeholders stay.
fn prune_by_size(s: &SpaceStore) -> rusqlite::Result<u64> {
    let used = used_bytes(s)?;
    if used <= KEEP_BYTES { return Ok(0); }
    let sizes: Vec<(Vec<u8>, u64)> = {
        let mut st = s.conn.prepare(&format!(
            "SELECT device, COALESCE(SUM(size), 0) FROM entries WHERE created_ms > 0 AND gone = 0 AND kind NOT IN {KEEP_KINDS}              GROUP BY device"
        ))?;
        let rows = st.query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, i64>(1)? as u64)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let evictable: u64 = sizes.iter().map(|x| x.1).sum();
    let target = evictable.saturating_sub(used - TRIM_TO_BYTES);
    let cap = AUTHOR_KEEP_BYTES.min(fair_cap(&sizes.iter().map(|x| x.1).collect::<Vec<_>>(), target));
    let mut total = 0u64;
    for (device, bytes) in sizes {
        if bytes <= cap { continue; }
        let Ok(arr) = <[u8; 32]>::try_from(device.as_slice()) else { continue };
        let rows: Vec<(Vec<u8>, i64, i64)> = {
            let mut st = s.conn.prepare(&format!(
                "SELECT id, seq, size FROM entries WHERE device = ?1 AND created_ms > 0 AND gone = 0                  AND kind NOT IN {KEEP_KINDS} ORDER BY seq"
            ))?;
            let it = st.query_map(params![device], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            it.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let (mut remaining, mut newest) = (bytes, 0i64);
        let mut take = Vec::new();
        for (id, seq, size) in rows {
            if remaining <= cap { break; }
            remaining = remaining.saturating_sub(size as u64);
            newest = newest.max(seq);
            take.push(id);
        }
        if take.is_empty() { continue; }
        s.raise_floor(&DeviceKey(arr), newest as u64)?;
        for id in &take { total += s.conn.execute("DELETE FROM entries WHERE id = ?1", params![id])? as u64; }
    }
    Ok(total)
}

/// The largest per-author cap `L` with `sum(min(size, L)) <= target` (0 when even L = 0 does not fit, which
/// cannot happen since the sum is then 0).
fn fair_cap(sizes: &[u64], target: u64) -> u64 {
    let mut sorted = sizes.to_vec();
    sorted.sort_unstable();
    let n = sorted.len() as u64;
    let (mut used, mut prev) = (0u64, 0u64);
    for (i, &sz) in sorted.iter().enumerate() {
        let left = n - i as u64;
        // Everyone from here on is capped at `sz`: the total would be used + sz * left.
        if used + sz * left > target {
            return prev + (target - used - prev * left) / left;
        }
        used += sz;
        prev = sz;
    }
    u64::MAX
}
