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

use std::sync::{Arc, Condvar, Mutex};
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
    /// Test mode: blocks on a condvar until `advance()` meets the deadline.
    fn sleep(&self, duration: Duration);
}

/// Create the process-wide clock.
///
/// Returns `RealClock` in production. Phase 0 item 4 will check
/// `ATTEND_TEST_MODE` and return a `MockClock` instead.
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

/// Test clock: time only advances via explicit `advance()` calls.
///
/// `sleep(d)` blocks on a condvar until `now() >= sleep_start + d`. When
/// `advance()` bumps the internal time and broadcasts the condvar, sleeping
/// threads whose deadlines are met wake and return. This eliminates both
/// CPU spin and real wall-clock delay: threads proceed in lockstep with
/// harness-driven time.
#[derive(Debug, Clone)]
pub struct MockClock {
    inner: Arc<MockClockInner>,
}

#[derive(Debug)]
struct MockClockInner {
    state: Mutex<DateTime<Utc>>,
    condvar: Condvar,
}

impl MockClock {
    /// Create a mock clock starting at the given time.
    pub fn new(start: DateTime<Utc>) -> Self {
        Self {
            inner: Arc::new(MockClockInner {
                state: Mutex::new(start),
                condvar: Condvar::new(),
            }),
        }
    }

    /// Advance the clock by the given duration.
    ///
    /// Wakes all threads blocked in `sleep()` whose deadlines are now met.
    pub fn advance(&self, duration: Duration) {
        let mut guard = self.inner.state.lock().unwrap();
        *guard += duration;
        drop(guard);
        self.inner.condvar.notify_all();
    }
}

impl Clock for MockClock {
    fn now(&self) -> DateTime<Utc> {
        *self.inner.state.lock().unwrap()
    }

    fn sleep(&self, duration: Duration) {
        let mut guard = self.inner.state.lock().unwrap();
        let deadline = *guard + duration;
        while *guard < deadline {
            guard = self.inner.condvar.wait(guard).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

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

    /// Multiple advance() calls accumulate.
    #[test]
    fn mock_advance_accumulates() {
        let start = Utc::now();
        let clock = MockClock::new(start);

        clock.advance(Duration::from_secs(5));
        clock.advance(Duration::from_secs(3));

        assert_eq!(clock.now(), start + Duration::from_secs(8));
    }

    /// sleep(Duration::ZERO) returns immediately without blocking.
    #[test]
    fn mock_sleep_zero_returns_immediately() {
        let start = Utc::now();
        let clock = MockClock::new(start);
        clock.sleep(Duration::ZERO);
        assert_eq!(clock.now(), start);
    }

    /// sleep() blocks until advance() meets the deadline, and does not
    /// itself move time forward.
    #[test]
    fn mock_sleep_blocks_until_deadline() {
        let start = Utc::now();
        let clock = MockClock::new(start);
        let clock2 = clock.clone();

        let handle = std::thread::spawn(move || {
            clock2.sleep(Duration::from_secs(10));
            clock2.now()
        });

        // Let the thread enter the condvar wait.
        std::thread::sleep(Duration::from_millis(50));
        clock.advance(Duration::from_secs(10));

        let woke_at = handle.join().unwrap();
        // Time is exactly what we advanced — sleep didn't add anything.
        assert_eq!(woke_at, start + Duration::from_secs(10));
    }

    /// A partial advance doesn't wake a thread whose deadline isn't met.
    #[test]
    fn mock_sleep_partial_advance_stays_blocked() {
        let start = Utc::now();
        let clock = MockClock::new(start);
        let clock2 = clock.clone();
        let woke = Arc::new(AtomicBool::new(false));
        let woke2 = Arc::clone(&woke);

        std::thread::spawn(move || {
            clock2.sleep(Duration::from_secs(10));
            woke2.store(true, Ordering::SeqCst);
        });

        std::thread::sleep(Duration::from_millis(50));
        clock.advance(Duration::from_secs(5));

        // Thread needs 10s but only 5s have passed — still blocked.
        std::thread::sleep(Duration::from_millis(50));
        assert!(!woke.load(Ordering::SeqCst));

        // Remaining 5s meets the deadline.
        clock.advance(Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(50));
        assert!(woke.load(Ordering::SeqCst));
    }

    /// Multiple threads sleeping with different deadlines wake independently.
    #[test]
    fn mock_sleep_multiple_threads_different_deadlines() {
        let start = Utc::now();
        let clock = MockClock::new(start);

        let c1 = clock.clone();
        let c2 = clock.clone();

        let h1 = std::thread::spawn(move || {
            c1.sleep(Duration::from_secs(5));
            c1.now()
        });
        let h2 = std::thread::spawn(move || {
            c2.sleep(Duration::from_secs(10));
            c2.now()
        });

        std::thread::sleep(Duration::from_millis(50));
        clock.advance(Duration::from_secs(5));

        let t1 = h1.join().unwrap();
        assert_eq!(t1, start + Duration::from_secs(5));

        // h2 should still be blocked; advance the remaining 5s.
        std::thread::sleep(Duration::from_millis(50));
        clock.advance(Duration::from_secs(5));

        let t2 = h2.join().unwrap();
        assert_eq!(t2, start + Duration::from_secs(10));
    }
}
