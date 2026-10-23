//! The block file one series lives in.
//!
//! ```text
//! header  16 B   "TPH1" · version u16 · kind u8 · 0 · record size u16 · 6 × 0
//! blocks         zstd-compressed runs of records, back to back
//! index   36 B   per block: first t_ns i64 · last t_ns i64 · records u32 · offset u64 · len u32 · crc32 u32
//! footer  24 B   index offset u64 · block count u32 · record count u64 · "TPHE"
//! ```
//!
//! Records are in time order (ties keep arrival order). A block holds one clock minute, capped at
//! [`MAX_BLOCK_RECORDS`], so seeking to a time decompresses one minute. Each block's CRC is over its
//! compressed bytes and is checked on every read: a damaged file fails loudly, it never replays
//! wrong prices. The writer builds `<path>.tmp` and renames it into place on [`Writer::finish`], so
//! a crash or an error leaves no partial file behind.

use crate::record::Record;
use crate::HistoryError;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"TPH1";
const END_MAGIC: &[u8; 4] = b"TPHE";
const VERSION: u16 = 1;
const HEADER: usize = 16;
const INDEX_ENTRY: usize = 36;
const FOOTER: usize = 24;
pub const MAX_BLOCK_RECORDS: usize = 65_536;
const ZSTD_LEVEL: i32 = 9;
const MINUTE_NS: i64 = 60_000_000_000;

/// One block's index entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockMeta {
    pub first_ns: i64,
    pub last_ns: i64,
    pub records: u32,
    offset: u64,
    len: u32,
    crc: u32,
}

fn minute_of(t_ns: i64) -> i64 {
    t_ns.div_euclid(MINUTE_NS)
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".tmp");
    PathBuf::from(s)
}

/// Writes one series. Push records in time order, then [`finish`](Writer::finish).
pub struct Writer<R: Record> {
    path: PathBuf,
    tmp: PathBuf,
    out: Option<BufWriter<File>>,
    pos: u64,
    block: Vec<u8>,
    block_n: usize,
    block_first: i64,
    block_last: i64,
    index: Vec<BlockMeta>,
    total: u64,
    last_ns: Option<i64>,
    _r: PhantomData<R>,
}

impl<R: Record> Writer<R> {
    pub fn create(path: &Path) -> Result<Writer<R>, HistoryError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = tmp_path(path);
        let mut out = BufWriter::new(File::create(&tmp)?);
        let mut h = [0u8; HEADER];
        h[..4].copy_from_slice(MAGIC);
        h[4..6].copy_from_slice(&VERSION.to_le_bytes());
        h[6] = R::KIND;
        h[8..10].copy_from_slice(&(R::SIZE as u16).to_le_bytes());
        out.write_all(&h)?;
        Ok(Writer {
            path: path.to_path_buf(),
            tmp,
            out: Some(out),
            pos: HEADER as u64,
            block: Vec::new(),
            block_n: 0,
            block_first: 0,
            block_last: 0,
            index: Vec::new(),
            total: 0,
            last_ns: None,
            _r: PhantomData,
        })
    }

    pub fn push(&mut self, r: &R) -> Result<(), HistoryError> {
        let t = r.time_ns();
        if let Some(prev) = self.last_ns {
            if t < prev {
                return Err(HistoryError::OutOfOrder { prev_ns: prev, next_ns: t });
            }
        }
        self.last_ns = Some(t);
        if self.block_n > 0 && (minute_of(t) != minute_of(self.block_first) || self.block_n >= MAX_BLOCK_RECORDS) {
            self.flush_block()?;
        }
        if self.block_n == 0 {
            self.block_first = t;
        }
        let at = self.block.len();
        self.block.resize(at + R::SIZE, 0);
        r.encode(&mut self.block[at..]);
        self.block_n += 1;
        self.block_last = t;
        self.total += 1;
        Ok(())
    }

    fn flush_block(&mut self) -> Result<(), HistoryError> {
        if self.block_n == 0 {
            return Ok(());
        }
        let comp = zstd::bulk::compress(&self.block, ZSTD_LEVEL)?;
        let out = self.out.as_mut().expect("writer open");
        out.write_all(&comp)?;
        self.index.push(BlockMeta {
            first_ns: self.block_first,
            last_ns: self.block_last,
            records: self.block_n as u32,
            offset: self.pos,
            len: comp.len() as u32,
            crc: crc32fast::hash(&comp),
        });
        self.pos += comp.len() as u64;
        self.block.clear();
        self.block_n = 0;
        Ok(())
    }

    /// Write the index and footer, sync, and move the file into place. Returns the record count.
    pub fn finish(mut self) -> Result<u64, HistoryError> {
        self.flush_block()?;
        let index_offset = self.pos;
        let mut out = self.out.take().expect("writer open");
        for m in &self.index {
            let mut e = [0u8; INDEX_ENTRY];
            e[0..8].copy_from_slice(&m.first_ns.to_le_bytes());
            e[8..16].copy_from_slice(&m.last_ns.to_le_bytes());
            e[16..20].copy_from_slice(&m.records.to_le_bytes());
            e[20..28].copy_from_slice(&m.offset.to_le_bytes());
            e[28..32].copy_from_slice(&m.len.to_le_bytes());
            e[32..36].copy_from_slice(&m.crc.to_le_bytes());
            out.write_all(&e)?;
        }
        let mut f = [0u8; FOOTER];
        f[0..8].copy_from_slice(&index_offset.to_le_bytes());
        f[8..12].copy_from_slice(&(self.index.len() as u32).to_le_bytes());
        f[12..20].copy_from_slice(&self.total.to_le_bytes());
        f[20..24].copy_from_slice(END_MAGIC);
        out.write_all(&f)?;
        let file = out.into_inner().map_err(|e| HistoryError::Io(e.to_string()))?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&self.tmp, &self.path)?;
        Ok(self.total)
    }
}

impl<R: Record> Drop for Writer<R> {
    /// An unfinished writer (error, panic, early return) leaves nothing behind.
    fn drop(&mut self) {
        if self.out.take().is_some() {
            let _ = std::fs::remove_file(&self.tmp);
        }
    }
}

/// Reads one series.
pub struct Reader<R: Record> {
    path: PathBuf,
    file: File,
    index: Vec<BlockMeta>,
    total: u64,
    _r: PhantomData<R>,
}

impl<R: Record> Reader<R> {
    pub fn open(path: &Path) -> Result<Reader<R>, HistoryError> {
        let shown = path.display().to_string();
        let bad = |m: &str| HistoryError::Format(format!("{shown}: {m}"));
        let mut file = File::open(path).map_err(|e| HistoryError::Io(format!("{shown}: {e}")))?;
        let len = file.metadata()?.len();
        if len < (HEADER + FOOTER) as u64 {
            return Err(bad("too short"));
        }
        let mut h = [0u8; HEADER];
        file.read_exact(&mut h)?;
        if &h[..4] != MAGIC {
            return Err(bad("wrong magic"));
        }
        let version = u16::from_le_bytes([h[4], h[5]]);
        if version != VERSION {
            return Err(bad(&format!("version {version}, this build reads {VERSION}")));
        }
        if h[6] != R::KIND {
            return Err(bad(&format!("holds kind {}, asked for kind {}", h[6], R::KIND)));
        }
        if u16::from_le_bytes([h[8], h[9]]) as usize != R::SIZE {
            return Err(bad("record size mismatch"));
        }
        let mut f = [0u8; FOOTER];
        file.seek(SeekFrom::Start(len - FOOTER as u64))?;
        file.read_exact(&mut f)?;
        if &f[20..24] != END_MAGIC {
            return Err(bad("no footer (unfinished write?)"));
        }
        let index_offset = u64::from_le_bytes(f[0..8].try_into().expect("8"));
        let blocks = u32::from_le_bytes(f[8..12].try_into().expect("4")) as usize;
        let total = u64::from_le_bytes(f[12..20].try_into().expect("8"));
        if index_offset + (blocks * INDEX_ENTRY) as u64 + FOOTER as u64 != len {
            return Err(bad("index does not fit the file"));
        }
        let mut raw = vec![0u8; blocks * INDEX_ENTRY];
        file.seek(SeekFrom::Start(index_offset))?;
        file.read_exact(&mut raw)?;
        let index = raw
            .chunks_exact(INDEX_ENTRY)
            .map(|e| BlockMeta {
                first_ns: i64::from_le_bytes(e[0..8].try_into().expect("8")),
                last_ns: i64::from_le_bytes(e[8..16].try_into().expect("8")),
                records: u32::from_le_bytes(e[16..20].try_into().expect("4")),
                offset: u64::from_le_bytes(e[20..28].try_into().expect("8")),
                len: u32::from_le_bytes(e[28..32].try_into().expect("4")),
                crc: u32::from_le_bytes(e[32..36].try_into().expect("4")),
            })
            .collect();
        Ok(Reader { path: path.to_path_buf(), file, index, total, _r: PhantomData })
    }

    pub fn records(&self) -> u64 {
        self.total
    }

    pub fn blocks(&self) -> &[BlockMeta] {
        &self.index
    }

    /// The first block holding a record stamped after `t_ns`, or `blocks().len()` if none.
    pub fn first_block_after(&self, t_ns: i64) -> usize {
        self.index.partition_point(|m| m.last_ns <= t_ns)
    }

    pub fn read_block(&mut self, i: usize) -> Result<Vec<R>, HistoryError> {
        let m = self.index[i];
        let mut comp = vec![0u8; m.len as usize];
        self.file.seek(SeekFrom::Start(m.offset))?;
        self.file.read_exact(&mut comp)?;
        if crc32fast::hash(&comp) != m.crc {
            return Err(HistoryError::Corrupt { path: self.path.display().to_string(), block: i });
        }
        let raw = zstd::bulk::decompress(&comp, m.records as usize * R::SIZE)?;
        if raw.len() != m.records as usize * R::SIZE {
            return Err(HistoryError::Corrupt { path: self.path.display().to_string(), block: i });
        }
        Ok(raw.chunks_exact(R::SIZE).map(R::decode).collect())
    }

    /// Every record. For small series and tests; replay uses [`crate::cursor::Cursor`].
    pub fn read_all(&mut self) -> Result<Vec<R>, HistoryError> {
        let mut out = Vec::with_capacity(self.total as usize);
        for i in 0..self.index.len() {
            out.extend(self.read_block(i)?);
        }
        Ok(out)
    }
}

/// Write `records` (already in time order) to `path` in one go.
pub fn write_all<R: Record>(path: &Path, records: &[R]) -> Result<u64, HistoryError> {
    let mut w = Writer::<R>::create(path)?;
    for r in records {
        w.push(r)?;
    }
    w.finish()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::record::{Quote, Trade};

    /// A fresh directory per call: tests run in parallel and must never share one.
    pub fn temp_dir(name: &str) -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!("tp-history-test-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    pub fn trade(t_ns: i64, price: f64) -> Trade {
        Trade { t_ns, price, size: 100, exchange: 1, flags: 0, contract: 0 }
    }

    const S: i64 = 1_000_000_000;

    #[test]
    fn writes_and_reads_back_split_by_minute() {
        let d = temp_dir("rw");
        let p = d.join("x.trades");
        // 3 records in minute 0, 2 in minute 1, 1 in minute 5.
        let recs = vec![trade(S, 1.0), trade(2 * S, 1.1), trade(59 * S, 1.2), trade(60 * S, 1.3), trade(61 * S, 1.4), trade(300 * S, 1.5)];
        assert_eq!(write_all(&p, &recs).unwrap(), 6);
        let mut r = Reader::<Trade>::open(&p).unwrap();
        assert_eq!(r.records(), 6);
        assert_eq!(r.blocks().len(), 3);
        assert_eq!(r.read_all().unwrap(), recs);
        assert!(!tmp_path(&p).exists(), "tmp file must be renamed away");
    }

    #[test]
    fn finds_the_block_after_a_time() {
        let d = temp_dir("seek");
        let p = d.join("x.trades");
        write_all(&p, &[trade(S, 1.0), trade(60 * S, 2.0), trade(120 * S, 3.0)]).unwrap();
        let r = Reader::<Trade>::open(&p).unwrap();
        assert_eq!(r.first_block_after(0), 0);
        assert_eq!(r.first_block_after(S), 1, "a record AT t is not after t");
        assert_eq!(r.first_block_after(90 * S), 2);
        assert_eq!(r.first_block_after(120 * S), 3);
    }

    #[test]
    fn large_minutes_split_at_the_cap() {
        let d = temp_dir("cap");
        let p = d.join("x.trades");
        let recs: Vec<Trade> = (0..(MAX_BLOCK_RECORDS + 10)).map(|i| trade(i as i64, 1.0)).collect();
        write_all(&p, &recs).unwrap();
        let mut r = Reader::<Trade>::open(&p).unwrap();
        assert_eq!(r.blocks().len(), 2);
        assert_eq!(r.read_all().unwrap().len(), recs.len());
    }

    #[test]
    fn out_of_order_is_refused_and_leaves_no_file() {
        let d = temp_dir("order");
        let p = d.join("x.trades");
        let mut w = Writer::<Trade>::create(&p).unwrap();
        w.push(&trade(10, 1.0)).unwrap();
        assert!(matches!(w.push(&trade(9, 1.0)), Err(HistoryError::OutOfOrder { .. })));
        drop(w);
        assert!(!p.exists());
        assert!(!tmp_path(&p).exists());
    }

    #[test]
    fn equal_times_keep_arrival_order() {
        let d = temp_dir("ties");
        let p = d.join("x.trades");
        let recs = vec![trade(5, 1.0), trade(5, 2.0), trade(5, 3.0)];
        write_all(&p, &recs).unwrap();
        assert_eq!(Reader::<Trade>::open(&p).unwrap().read_all().unwrap(), recs);
    }

    #[test]
    fn damage_is_detected() {
        let d = temp_dir("crc");
        let p = d.join("x.trades");
        write_all(&p, &[trade(S, 1.0), trade(2 * S, 2.0)]).unwrap();
        let mut bytes = std::fs::read(&p).unwrap();
        bytes[HEADER + 3] ^= 0xFF; // inside the first block
        std::fs::write(&p, bytes).unwrap();
        let mut r = Reader::<Trade>::open(&p).unwrap();
        assert!(matches!(r.read_block(0), Err(HistoryError::Corrupt { block: 0, .. })));
    }

    #[test]
    fn the_wrong_kind_is_refused() {
        let d = temp_dir("kind");
        let p = d.join("x.trades");
        write_all(&p, &[trade(1, 1.0)]).unwrap();
        assert!(matches!(Reader::<Quote>::open(&p), Err(HistoryError::Format(_))));
    }

    #[test]
    fn an_unfinished_file_is_refused() {
        let d = temp_dir("trunc");
        let p = d.join("x.trades");
        write_all(&p, &[trade(1, 1.0), trade(2, 2.0)]).unwrap();
        let bytes = std::fs::read(&p).unwrap();
        std::fs::write(&p, &bytes[..bytes.len() - 3]).unwrap();
        assert!(Reader::<Trade>::open(&p).is_err());
    }

    #[test]
    fn an_empty_series_is_valid() {
        let d = temp_dir("empty");
        let p = d.join("x.trades");
        assert_eq!(write_all::<Trade>(&p, &[]).unwrap(), 0);
        let mut r = Reader::<Trade>::open(&p).unwrap();
        assert_eq!(r.records(), 0);
        assert!(r.read_all().unwrap().is_empty());
        assert_eq!(r.first_block_after(0), 0);
    }
}
