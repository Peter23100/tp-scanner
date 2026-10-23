//! Reading a series against a clock — the one place look-ahead is prevented.
//!
//! A [`Cursor`] is positioned at a time and only ever moves forward: [`Cursor::advance_to`] hands
//! back every record stamped in `(previous time, now]` and nothing later. Going back in time is a
//! [`Cursor::seek`], which drops everything already handed out (the backtest resets its session on
//! a backward jump for the same reason: nothing may exist before it happened).
//!
//! [`state_at`] rebuilds "what was showing at time t" — the last record per contract at or before
//! `t` — for the moment a replay starts or jumps.

use crate::file::Reader;
use crate::record::Record;
use crate::HistoryError;
use std::collections::HashMap;

pub struct Cursor<R: Record> {
    reader: Reader<R>,
    /// The next block to load.
    next_block: usize,
    buf: Vec<R>,
    pos: usize,
    now_ns: i64,
}

impl<R: Record> Cursor<R> {
    /// A cursor at `t_ns`: its first `advance_to` yields records stamped after `t_ns`.
    pub fn new(reader: Reader<R>, t_ns: i64) -> Result<Cursor<R>, HistoryError> {
        let mut c = Cursor { reader, next_block: 0, buf: Vec::new(), pos: 0, now_ns: t_ns };
        c.seek(t_ns)?;
        Ok(c)
    }

    /// Where the cursor stands: every record at or before this time has been handed out.
    pub fn now_ns(&self) -> i64 {
        self.now_ns
    }

    /// Move to `t_ns`, forward or back, without handing out what lies between.
    pub fn seek(&mut self, t_ns: i64) -> Result<(), HistoryError> {
        self.now_ns = t_ns;
        self.next_block = self.reader.first_block_after(t_ns);
        self.buf.clear();
        self.pos = 0;
        if self.next_block < self.reader.blocks().len() {
            self.buf = self.reader.read_block(self.next_block)?;
            self.next_block += 1;
            self.pos = self.buf.partition_point(|r| r.time_ns() <= t_ns);
        }
        Ok(())
    }

    /// Every record stamped in `(now, to]`, in order; the cursor then stands at `to`. Asking for an
    /// earlier time than the cursor's is an error, not an empty answer: it means the caller's
    /// clock went backwards and it should [`seek`](Cursor::seek).
    pub fn advance_to(&mut self, to_ns: i64) -> Result<Vec<R>, HistoryError> {
        if to_ns < self.now_ns {
            return Err(HistoryError::Format(format!("cursor asked to go back from {} to {}; seek instead", self.now_ns, to_ns)));
        }
        let mut out = Vec::new();
        loop {
            while self.pos < self.buf.len() {
                let r = &self.buf[self.pos];
                if r.time_ns() > to_ns {
                    self.now_ns = to_ns;
                    return Ok(out);
                }
                out.push(r.clone());
                self.pos += 1;
            }
            let blocks = self.reader.blocks();
            if self.next_block >= blocks.len() || blocks[self.next_block].first_ns > to_ns {
                self.now_ns = to_ns;
                return Ok(out);
            }
            self.buf = self.reader.read_block(self.next_block)?;
            self.next_block += 1;
            self.pos = 0;
        }
    }

    /// When the next record will become knowable, if there is one. Lets a MAX-speed driver jump
    /// straight to the next event instead of stepping through empty time.
    pub fn peek_next_ns(&mut self) -> Result<Option<i64>, HistoryError> {
        if self.pos < self.buf.len() {
            return Ok(Some(self.buf[self.pos].time_ns()));
        }
        Ok(self.reader.blocks().get(self.next_block).map(|m| m.first_ns))
    }
}

/// The last record of each contract stamped at or before `t_ns` — what a screen showed at that
/// moment. Reads only blocks that start at or before `t_ns`.
pub fn state_at<R: Record>(reader: &mut Reader<R>, t_ns: i64) -> Result<HashMap<u32, R>, HistoryError> {
    let mut out = HashMap::new();
    let n = reader.blocks().partition_point(|m| m.first_ns <= t_ns);
    for i in 0..n {
        for r in reader.read_block(i)? {
            if r.time_ns() > t_ns {
                break;
            }
            out.insert(r.contract(), r);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file::tests::{temp_dir, trade};
    use crate::file::write_all;
    use crate::record::Trade;

    const S: i64 = 1_000_000_000;

    fn series(name: &str, times: &[i64]) -> Reader<Trade> {
        let d = temp_dir(name);
        let p = d.join("x.trades");
        let recs: Vec<Trade> = times.iter().map(|&t| trade(t, t as f64)).collect();
        write_all(&p, &recs).unwrap();
        Reader::open(&p).unwrap()
    }

    fn times(v: &[Trade]) -> Vec<i64> {
        v.iter().map(|r| r.t_ns).collect()
    }

    #[test]
    fn advance_hands_out_each_record_once_and_never_early() {
        let all = [S, 30 * S, 60 * S, 61 * S, 200 * S, 201 * S];
        let mut c = Cursor::new(series("adv", &all), 0).unwrap();
        assert_eq!(times(&c.advance_to(30 * S).unwrap()), vec![S, 30 * S], "a record AT now is knowable");
        assert_eq!(times(&c.advance_to(60 * S).unwrap()), vec![60 * S]);
        assert!(c.advance_to(100 * S).unwrap().len() == 1);
        assert!(c.advance_to(100 * S).unwrap().is_empty(), "standing still yields nothing new");
        assert_eq!(times(&c.advance_to(1_000 * S).unwrap()), vec![200 * S, 201 * S]);
        assert!(c.advance_to(2_000 * S).unwrap().is_empty());
    }

    #[test]
    fn nothing_is_ever_later_than_now() {
        // Every record from 0..600 s, advanced in uneven steps across block boundaries.
        let all: Vec<i64> = (0..600).map(|i| i * S + 17).collect();
        let mut c = Cursor::new(series("noahead", &all), -1).unwrap();
        let mut seen = 0;
        let mut now = 0;
        for step in [1, 7, 59, 61, 3, 120, 1, 400] {
            now += step * S;
            for r in c.advance_to(now).unwrap() {
                assert!(r.t_ns <= now, "record {} handed out at {now}", r.t_ns);
                seen += 1;
            }
        }
        assert_eq!(seen, all.iter().filter(|&&t| t <= now).count());
    }

    #[test]
    fn starting_mid_day_skips_the_past() {
        let mut c = Cursor::new(series("start", &[S, 61 * S, 62 * S, 125 * S]), 61 * S).unwrap();
        assert_eq!(times(&c.advance_to(130 * S).unwrap()), vec![62 * S, 125 * S]);
    }

    #[test]
    fn going_back_is_an_error_and_seek_works_both_ways() {
        let mut c = Cursor::new(series("seek", &[S, 61 * S, 121 * S]), 0).unwrap();
        c.advance_to(100 * S).unwrap();
        assert!(c.advance_to(50 * S).is_err());
        c.seek(50 * S).unwrap();
        assert_eq!(times(&c.advance_to(200 * S).unwrap()), vec![61 * S, 121 * S]);
        c.seek(0).unwrap();
        assert_eq!(times(&c.advance_to(200 * S).unwrap()).len(), 3);
    }

    #[test]
    fn peek_finds_the_next_event() {
        let mut c = Cursor::new(series("peek", &[S, 500 * S]), 0).unwrap();
        assert_eq!(c.peek_next_ns().unwrap(), Some(S));
        c.advance_to(S).unwrap();
        assert_eq!(c.peek_next_ns().unwrap(), Some(500 * S));
        c.advance_to(500 * S).unwrap();
        assert_eq!(c.peek_next_ns().unwrap(), None);
    }

    #[test]
    fn state_at_keeps_the_last_record_per_contract() {
        let d = temp_dir("state");
        let p = d.join("x.trades");
        let mk = |t: i64, c: u32, px: f64| Trade { t_ns: t, price: px, size: 1, exchange: 0, flags: 0, contract: c };
        write_all(&p, &[mk(S, 1, 1.0), mk(2 * S, 2, 5.0), mk(70 * S, 1, 1.5), mk(130 * S, 1, 9.9)]).unwrap();
        let mut r = Reader::<Trade>::open(&p).unwrap();
        let st = state_at(&mut r, 100 * S).unwrap();
        assert_eq!(st[&1].price, 1.5);
        assert_eq!(st[&2].price, 5.0);
        assert!(state_at(&mut r, 0).unwrap().is_empty());
    }
}
