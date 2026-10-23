//! The replay clock: sim time from wall time.
//!
//! Pure arithmetic — wall time is passed in (monotonic ns from any origin), so the clock is exact
//! and testable, and the replay never reads the machine's wall clock for market time.
//!
//! - At a speed `s`, sim time runs `s` × wall time from the last anchor.
//! - [`Pace::Max`] doesn't run on its own: the driver moves it with [`ReplayClock::jump_to`] to the
//!   next event as fast as it can serve them.
//! - Changing speed, pausing and resuming re-anchor, so sim time never jumps when they happen.
//! - Sim time stops at `end_ns` (the session's end), if set.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pace {
    /// Sim seconds per wall second. 1.0 = real time.
    Speed(f64),
    /// As fast as the data can be served.
    Max,
}

#[derive(Clone, Debug)]
pub struct ReplayClock {
    anchor_wall_ns: i64,
    anchor_sim_ns: i64,
    pace: Pace,
    paused: bool,
    end_ns: Option<i64>,
}

impl ReplayClock {
    pub fn new(start_sim_ns: i64, wall_ns: i64, pace: Pace) -> ReplayClock {
        ReplayClock { anchor_wall_ns: wall_ns, anchor_sim_ns: start_sim_ns, pace, paused: false, end_ns: None }
    }

    pub fn with_end(mut self, end_ns: i64) -> ReplayClock {
        self.end_ns = Some(end_ns);
        self
    }

    pub fn pace(&self) -> Pace {
        self.pace
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// True once sim time has reached the end (if one is set).
    pub fn is_finished(&self, wall_ns: i64) -> bool {
        self.end_ns.is_some_and(|e| self.now(wall_ns) >= e)
    }

    pub fn now(&self, wall_ns: i64) -> i64 {
        let t = match (self.paused, self.pace) {
            (false, Pace::Speed(s)) => {
                let elapsed = (wall_ns - self.anchor_wall_ns).max(0) as f64;
                self.anchor_sim_ns + (elapsed * s) as i64
            }
            _ => self.anchor_sim_ns,
        };
        match self.end_ns {
            Some(e) => t.min(e),
            None => t,
        }
    }

    fn reanchor(&mut self, wall_ns: i64) {
        self.anchor_sim_ns = self.now(wall_ns);
        self.anchor_wall_ns = wall_ns;
    }

    pub fn set_pace(&mut self, pace: Pace, wall_ns: i64) {
        self.reanchor(wall_ns);
        self.pace = pace;
    }

    pub fn pause(&mut self, wall_ns: i64) {
        self.reanchor(wall_ns);
        self.paused = true;
    }

    pub fn resume(&mut self, wall_ns: i64) {
        self.anchor_wall_ns = wall_ns;
        self.paused = false;
    }

    /// Move sim time to `sim_ns`, forward or back. Callers decide what a backward jump means for
    /// open orders and positions (the backtest resets its session).
    pub fn jump_to(&mut self, sim_ns: i64, wall_ns: i64) {
        self.anchor_sim_ns = match self.end_ns {
            Some(e) => sim_ns.min(e),
            None => sim_ns,
        };
        self.anchor_wall_ns = wall_ns;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: i64 = 1_000_000_000;

    #[test]
    fn runs_at_its_speed() {
        let c = ReplayClock::new(1_000 * S, 0, Pace::Speed(60.0));
        assert_eq!(c.now(0), 1_000 * S);
        assert_eq!(c.now(S), 1_060 * S, "one wall second at 60x is one sim minute");
        assert_eq!(c.now(-5 * S), 1_000 * S, "wall time before the anchor never runs it backwards");
    }

    #[test]
    fn pause_freezes_and_resume_continues_without_a_jump() {
        let mut c = ReplayClock::new(0, 0, Pace::Speed(10.0));
        c.pause(2 * S);
        assert_eq!(c.now(2 * S), 20 * S);
        assert_eq!(c.now(100 * S), 20 * S);
        c.resume(100 * S);
        assert_eq!(c.now(100 * S), 20 * S);
        assert_eq!(c.now(101 * S), 30 * S);
    }

    #[test]
    fn changing_speed_keeps_time_continuous() {
        let mut c = ReplayClock::new(0, 0, Pace::Speed(1.0));
        c.set_pace(Pace::Speed(300.0), 10 * S);
        assert_eq!(c.now(10 * S), 10 * S);
        assert_eq!(c.now(11 * S), 310 * S);
    }

    #[test]
    fn max_moves_only_when_the_driver_jumps() {
        let mut c = ReplayClock::new(5 * S, 0, Pace::Max);
        assert_eq!(c.now(1_000 * S), 5 * S);
        c.jump_to(90 * S, 1_000 * S);
        assert_eq!(c.now(1_001 * S), 90 * S);
    }

    #[test]
    fn stops_at_the_end() {
        let mut c = ReplayClock::new(0, 0, Pace::Speed(100.0)).with_end(50 * S);
        assert_eq!(c.now(S), 50 * S);
        assert!(c.is_finished(S));
        c.jump_to(80 * S, S);
        assert_eq!(c.now(S), 50 * S);
    }

    #[test]
    fn jumping_back_is_allowed() {
        let mut c = ReplayClock::new(100 * S, 0, Pace::Speed(1.0));
        c.jump_to(10 * S, 5 * S);
        assert_eq!(c.now(6 * S), 11 * S);
    }
}
