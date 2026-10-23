//! The record types and their fixed-width little-endian encoding.
//!
//! Every record carries `contract`: 0 for a stock's own series, and for options the id of the
//! contract within its underlying's day (see [`crate::store::ContractTable`]). One file then holds
//! every contract of an underlying for the day instead of thousands of tiny files.

/// A fixed-width record a history file can hold.
pub trait Record: Sized + Clone {
    /// Which series kind this is; written into the file header and checked on open.
    const KIND: u8;
    /// Encoded width in bytes.
    const SIZE: usize;
    /// The time the record became knowable, UTC ns. Files are sorted by it.
    fn time_ns(&self) -> i64;
    fn contract(&self) -> u32;
    /// Write exactly `SIZE` bytes into `out`.
    fn encode(&self, out: &mut [u8]);
    /// Read from exactly `SIZE` bytes.
    fn decode(b: &[u8]) -> Self;
}

/// One print.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trade {
    /// SIP timestamp, UTC ns.
    pub t_ns: i64,
    pub price: f64,
    pub size: u32,
    /// Massive/SIP exchange id; 0 = unknown.
    pub exchange: u8,
    /// Bit flags: [`Trade::IRREGULAR`] = condition codes say it doesn't update last/high/low
    /// (odd lot, out of sequence, average price…); [`Trade::CORRECTION`] = corrected or cancelled.
    pub flags: u8,
    pub contract: u32,
}

impl Trade {
    pub const IRREGULAR: u8 = 1;
    pub const CORRECTION: u8 = 2;

    /// True if a fill engine may treat this print as the market trading at `price`.
    pub fn is_regular(&self) -> bool {
        self.flags & (Self::IRREGULAR | Self::CORRECTION) == 0
    }
}

/// One top-of-book (NBBO) update. A side with no quote has price 0 and size 0 and must read as
/// unknown, never as a price of zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quote {
    pub t_ns: i64,
    pub bid: f64,
    pub ask: f64,
    pub bid_size: u32,
    pub ask_size: u32,
    pub bid_exchange: u8,
    pub ask_exchange: u8,
    pub contract: u32,
}

impl Quote {
    pub fn bid(&self) -> Option<f64> {
        (self.bid > 0.0 && self.bid_size > 0).then_some(self.bid)
    }
    pub fn ask(&self) -> Option<f64> {
        (self.ask > 0.0 && self.ask_size > 0).then_some(self.ask)
    }
}

/// An aggregate bar. `t_ns` is the bar's **end** (start + width): the moment it became knowable.
/// Storing the end, not the start, is what keeps a bar from being served one bar early.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar {
    pub t_ns: i64,
    /// Width in seconds (1 for second bars, 60 for minute bars).
    pub width_s: u32,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub contract: u32,
}

impl Bar {
    pub fn start_ns(&self) -> i64 {
        self.t_ns - self.width_s as i64 * 1_000_000_000
    }
}

fn put_i64(b: &mut [u8], at: usize, v: i64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}
fn put_f64(b: &mut [u8], at: usize, v: f64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}
fn put_u32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn get_i64(b: &[u8], at: usize) -> i64 {
    i64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"))
}
fn get_f64(b: &[u8], at: usize) -> f64 {
    f64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"))
}
fn get_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}

impl Record for Trade {
    const KIND: u8 = 1;
    const SIZE: usize = 8 + 8 + 4 + 1 + 1 + 4;
    fn time_ns(&self) -> i64 {
        self.t_ns
    }
    fn contract(&self) -> u32 {
        self.contract
    }
    fn encode(&self, b: &mut [u8]) {
        put_i64(b, 0, self.t_ns);
        put_f64(b, 8, self.price);
        put_u32(b, 16, self.size);
        b[20] = self.exchange;
        b[21] = self.flags;
        put_u32(b, 22, self.contract);
    }
    fn decode(b: &[u8]) -> Self {
        Trade { t_ns: get_i64(b, 0), price: get_f64(b, 8), size: get_u32(b, 16), exchange: b[20], flags: b[21], contract: get_u32(b, 22) }
    }
}

impl Record for Quote {
    const KIND: u8 = 2;
    const SIZE: usize = 8 + 8 + 8 + 4 + 4 + 1 + 1 + 4;
    fn time_ns(&self) -> i64 {
        self.t_ns
    }
    fn contract(&self) -> u32 {
        self.contract
    }
    fn encode(&self, b: &mut [u8]) {
        put_i64(b, 0, self.t_ns);
        put_f64(b, 8, self.bid);
        put_f64(b, 16, self.ask);
        put_u32(b, 24, self.bid_size);
        put_u32(b, 28, self.ask_size);
        b[32] = self.bid_exchange;
        b[33] = self.ask_exchange;
        put_u32(b, 34, self.contract);
    }
    fn decode(b: &[u8]) -> Self {
        Quote {
            t_ns: get_i64(b, 0),
            bid: get_f64(b, 8),
            ask: get_f64(b, 16),
            bid_size: get_u32(b, 24),
            ask_size: get_u32(b, 28),
            bid_exchange: b[32],
            ask_exchange: b[33],
            contract: get_u32(b, 34),
        }
    }
}

impl Record for Bar {
    const KIND: u8 = 3;
    const SIZE: usize = 8 + 4 + 8 * 5 + 4;
    fn time_ns(&self) -> i64 {
        self.t_ns
    }
    fn contract(&self) -> u32 {
        self.contract
    }
    fn encode(&self, b: &mut [u8]) {
        put_i64(b, 0, self.t_ns);
        put_u32(b, 8, self.width_s);
        put_f64(b, 12, self.open);
        put_f64(b, 20, self.high);
        put_f64(b, 28, self.low);
        put_f64(b, 36, self.close);
        put_f64(b, 44, self.volume);
        put_u32(b, 52, self.contract);
    }
    fn decode(b: &[u8]) -> Self {
        Bar {
            t_ns: get_i64(b, 0),
            width_s: get_u32(b, 8),
            open: get_f64(b, 12),
            high: get_f64(b, 20),
            low: get_f64(b, 28),
            close: get_f64(b, 36),
            volume: get_f64(b, 44),
            contract: get_u32(b, 52),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip<R: Record + PartialEq + std::fmt::Debug>(r: R) {
        let mut b = vec![0u8; R::SIZE];
        r.encode(&mut b);
        assert_eq!(R::decode(&b), r);
    }

    #[test]
    fn every_record_roundtrips() {
        roundtrip(Trade { t_ns: 1_760_000_000_123_456_789, price: 216.0811, size: 25, exchange: 12, flags: Trade::IRREGULAR, contract: 7 });
        roundtrip(Quote { t_ns: -5, bid: 0.35, ask: 0.40, bid_size: 12, ask_size: 3, bid_exchange: 1, ask_exchange: 9, contract: u32::MAX });
        roundtrip(Bar { t_ns: 60_000_000_000, width_s: 60, open: 1.0, high: 2.0, low: 0.5, close: 1.5, volume: 1e9, contract: 0 });
    }

    #[test]
    fn kinds_are_distinct() {
        assert_ne!(Trade::KIND, Quote::KIND);
        assert_ne!(Quote::KIND, Bar::KIND);
        assert_ne!(Trade::KIND, Bar::KIND);
    }

    #[test]
    fn an_empty_side_is_unknown_not_zero() {
        let q = Quote { t_ns: 0, bid: 0.0, ask: 0.40, bid_size: 0, ask_size: 5, bid_exchange: 0, ask_exchange: 1, contract: 0 };
        assert_eq!(q.bid(), None);
        assert_eq!(q.ask(), Some(0.40));
    }

    #[test]
    fn irregular_and_corrected_prints_are_not_regular() {
        let t = Trade { t_ns: 0, price: 1.0, size: 1, exchange: 0, flags: 0, contract: 0 };
        assert!(t.is_regular());
        assert!(!Trade { flags: Trade::IRREGULAR, ..t }.is_regular());
        assert!(!Trade { flags: Trade::CORRECTION, ..t }.is_regular());
    }

    #[test]
    fn a_bar_is_stamped_at_its_end() {
        let b = Bar { t_ns: 120_000_000_000, width_s: 60, open: 0.0, high: 0.0, low: 0.0, close: 0.0, volume: 0.0, contract: 0 };
        assert_eq!(b.start_ns(), 60_000_000_000);
    }
}
