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

use std::sync::atomic::{AtomicUsize, Ordering};
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
/// Returns the `MockClock` from [`crate::test_mode`] if test mode is
/// active, otherwise a [`RealClock`].
pub fn process_clock() -> Arc<dyn Clock> {
    if let Some(clock) = crate::test_mode::clock() {
        clock
    } else {
        Arc::new(RealClock)
    }
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
    /// Number of threads currently blocked inside `sleep()`.
    waiters: AtomicUsize,
}

impl MockClock {
    /// Create a mock clock starting at the given time.
    pub fn new(start: DateTime<Utc>) -> Self {
        Self {
            inner: Arc::new(MockClockInner {
                state: Mutex::new(start),
                condvar: Condvar::new(),
                waiters: AtomicUsize::new(0),
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

    /// Spin-yield until at least `n` threads are blocked in `sleep()`.
    ///
    /// Used by test harnesses to synchronize with worker threads before
    /// calling `advance()`, replacing wall-clock sleeps with a
    /// deterministic rendezvous.
    pub fn wait_for_waiters(&self, n: usize) {
        while self.inner.waiters.load(Ordering::Acquire) < n {
            std::thread::yield_now();
        }
    }
}

impl Clock for MockClock {
    fn now(&self) -> DateTime<Utc> {
        *self.inner.state.lock().unwrap()
    }

    fn sleep(&self, duration: Duration) {
        let mut guard = self.inner.state.lock().unwrap();
        let deadline = *guard + duration;
        if *guard < deadline {
            self.inner.waiters.fetch_add(1, Ordering::Release);
            while *guard < deadline {
                guard = self.inner.condvar.wait(guard).unwrap();
            }
            self.inner.waiters.fetch_sub(1, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests;
