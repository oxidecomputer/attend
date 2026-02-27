//! Injectable clock for deterministic time in tests.
//!
//! Production code uses [`RealClock`], which delegates to `chrono::Utc::now()`
//! and `std::thread::sleep()`. Test mode substitutes a [`MockClock`] whose
//! time only advances via explicit `advance()` calls from the test harness.
//!
//! The clock replaces all uses of `Instant::now()`, `Utc::now()`, and
//! `thread::sleep()` in the daemon's capture and recording loops. This
//! eliminates wall-clock nondeterminism from tests entirely.
//!
//! Exceptions (not clock-gated):
//! - `chime.rs`: audio playback sleep is a no-op in test mode.
//! - `whisper.rs` / `parakeet.rs` bench functions: developer diagnostics.
//! - `transcribe.rs` model load timing: diagnostic logging.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};

/// Source of wall-clock time and delays.
///
/// All daemon loops and capture threads obtain timestamps and sleep
/// through this trait, making time fully controllable in tests.
pub trait Clock: Send + Sync {
    /// Current wall-clock time (UTC).
    fn now(&self) -> DateTime<Utc>;

    /// Sleep for the given duration.
    ///
    /// Production: real `thread::sleep()`.
    /// Test mode: returns immediately without advancing the clock.
    /// Time only advances via explicit `MockClock::advance()` calls.
    fn sleep(&self, duration: Duration);
}

/// Create the process-wide clock.
///
/// Returns `RealClock` in production. Phase 0 item 4 will check
/// `ATTEND_TEST_MODE` and return a `MockClock` instead.
#[allow(dead_code)] // Used once remaining CLI call sites are converted.
pub fn process_clock() -> Arc<dyn Clock> {
    Arc::new(RealClock)
}

/// Production clock: real wall-clock time and real sleep.
#[derive(Debug, Clone, Copy)]
pub struct RealClock;

impl Clock for RealClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// Test clock: time is frozen until explicitly advanced.
///
/// `sleep()` returns immediately (no wall-clock delay). Time only
/// moves forward via `advance()`, giving the test harness full control.
///
/// Currently gated behind `#[cfg(test)]`. Will be un-gated when
/// `ATTEND_TEST_MODE` (Phase 0, item 4) needs it in the production binary.
#[cfg(test)]
#[derive(Debug, Clone)]
pub struct MockClock {
    inner: std::sync::Arc<std::sync::Mutex<DateTime<Utc>>>,
}

#[cfg(test)]
impl MockClock {
    /// Create a mock clock starting at the given time.
    pub fn new(start: DateTime<Utc>) -> Self {
        Self {
            inner: std::sync::Arc::new(std::sync::Mutex::new(start)),
        }
    }

    /// Advance the clock by the given duration.
    pub fn advance(&self, duration: Duration) {
        let mut guard = self.inner.lock().unwrap();
        *guard += duration;
    }
}

#[cfg(test)]
impl Clock for MockClock {
    fn now(&self) -> DateTime<Utc> {
        *self.inner.lock().unwrap()
    }

    fn sleep(&self, _duration: Duration) {
        // No-op: test harness controls time via advance().
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RealClock::now() returns a time close to actual wall-clock.
    #[test]
    fn real_clock_returns_current_time() {
        let clock = RealClock;
        let before = Utc::now();
        let from_clock = clock.now();
        let after = Utc::now();
        assert!(from_clock >= before);
        assert!(from_clock <= after);
    }

    /// MockClock::now() returns the initial time until advanced.
    #[test]
    fn mock_clock_is_frozen_until_advanced() {
        let start = Utc::now();
        let clock = MockClock::new(start);

        assert_eq!(clock.now(), start);
        assert_eq!(clock.now(), start);

        clock.advance(Duration::from_secs(10));
        assert_eq!(clock.now(), start + Duration::from_secs(10));
    }

    /// MockClock::sleep() does not advance time.
    #[test]
    fn mock_sleep_does_not_advance_time() {
        let start = Utc::now();
        let clock = MockClock::new(start);

        clock.sleep(Duration::from_secs(60));
        assert_eq!(clock.now(), start);
    }

    /// Multiple advance() calls accumulate.
    #[test]
    fn mock_advance_accumulates() {
        let start = Utc::now();
        let clock = MockClock::new(start);

        clock.advance(Duration::from_secs(5));
        clock.advance(Duration::from_secs(3));

        assert_eq!(clock.now(), start + Duration::from_secs(8));
    }
}
