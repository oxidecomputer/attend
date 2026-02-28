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
use std::thread::JoinHandle;
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

    /// Create a per-thread clock for settlement tracking.
    ///
    /// The returned clock signals departure when dropped, so
    /// `advance_and_settle()` won't block waiting for a thread that has
    /// exited. Production clocks return a plain clone (no-op drop).
    fn for_thread(&self) -> Arc<dyn Clock> {
        Arc::new(RealClock)
    }
}

/// Spawn a thread that participates in the clock's settlement protocol.
///
/// The thread receives a per-thread clock via [`Clock::for_thread()`].
/// When the thread exits, its clock drops and departure is signaled,
/// so `advance_and_settle()` won't block waiting for a dead thread.
pub fn spawn_clock_thread<F, T>(name: &str, clock: &dyn Clock, f: F) -> JoinHandle<T>
where
    F: FnOnce(Arc<dyn Clock>) -> T + Send + 'static,
    T: Send + 'static,
{
    let thread_clock = clock.for_thread();
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || f(thread_clock))
        .expect("failed to spawn clock thread")
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

    fn for_thread(&self) -> Arc<dyn Clock> {
        Arc::new(RealClock)
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
    state: Mutex<ClockState>,
    condvar: Condvar,
    /// Settlement tracking: signaled when a thread re-enters `sleep()`
    /// after being woken by an advance. The reader thread's
    /// `advance_and_settle()` blocks on this until all woken threads
    /// have completed their per-tick work and re-entered `sleep()`.
    settlement: Condvar,
    settlement_state: Mutex<SettlementState>,
}

#[derive(Debug)]
struct ClockState {
    time: DateTime<Utc>,
    /// Deadlines of threads currently blocked in `sleep()`.
    /// Used by `advance_and_settle()` to count how many threads
    /// will wake, so settlement knows what to wait for.
    deadlines: Vec<DateTime<Utc>>,
}

#[derive(Debug, Default)]
struct SettlementState {
    /// Threads that have re-entered `sleep()` since the last advance.
    settled: usize,
    /// Threads that woke during the last advance (computed from deadlines).
    expected: usize,
    /// Total thread departures since clock creation (monotonically increasing).
    /// Incremented by `ParticipantMockClock::drop()`.
    departed: usize,
    /// Snapshot of `departed` at the start of the current `advance_and_settle`.
    /// Departures since this snapshot count toward settlement.
    departed_at_advance: usize,
}

impl MockClock {
    /// Create a mock clock starting at the given time.
    pub fn new(start: DateTime<Utc>) -> Self {
        Self {
            inner: Arc::new(MockClockInner {
                state: Mutex::new(ClockState {
                    time: start,
                    deadlines: Vec::new(),
                }),
                condvar: Condvar::new(),
                settlement: Condvar::new(),
                settlement_state: Mutex::new(SettlementState::default()),
            }),
        }
    }

    /// Block until at least `n` threads are blocked in `sleep()`.
    ///
    /// Uses the deadline registry: each thread in `sleep()` registers
    /// its deadline. This method waits on the time condvar (which
    /// `sleep()` notifies after registering) until `n` deadlines exist.
    pub fn wait_for_sleepers(&self, n: usize) {
        let mut state = self.inner.state.lock().unwrap();
        while state.deadlines.len() < n {
            state = self.inner.condvar.wait(state).unwrap();
        }
    }

    /// Advance the clock by the given duration.
    ///
    /// Wakes all threads blocked in `sleep()` whose deadlines are now
    /// met. Does NOT wait for settlement — use `advance_and_settle()`
    /// when you need the ACK protocol guarantee that woken threads
    /// have completed their work and re-entered `sleep()`.
    pub fn advance(&self, duration: Duration) {
        let mut state = self.inner.state.lock().unwrap();
        state.time += duration;
        drop(state);
        self.inner.condvar.notify_all();
    }

    /// Advance the clock and block until all woken threads have
    /// re-entered `sleep()`.
    ///
    /// This is the process-side primitive for the ACK protocol:
    /// the inject socket reader thread calls this on each `AdvanceTime`
    /// message, then sends `{"ack":true}` to the harness.
    ///
    /// Threads whose sleep deadline is met by the new time will wake,
    /// do their per-tick work, and call `sleep()` again. This method
    /// returns only after all such threads have re-entered `sleep()`,
    /// guaranteeing quiescence.
    pub fn advance_and_settle(&self, duration: Duration) {
        // Bump time and count how many threads will wake.
        let mut state = self.inner.state.lock().unwrap();
        state.time += duration;
        let woken = state
            .deadlines
            .iter()
            .filter(|dl| **dl <= state.time)
            .count();
        drop(state);

        // Reset settlement counter and snapshot departures before waking.
        let mut ss = self.inner.settlement_state.lock().unwrap();
        ss.expected = woken;
        ss.settled = 0;
        ss.departed_at_advance = ss.departed;
        drop(ss);

        // Wake all sleeping threads (those whose deadline isn't met
        // will re-block immediately without affecting settlement).
        self.inner.condvar.notify_all();

        if woken == 0 {
            return; // No threads to settle.
        }

        // Block until all woken threads have either re-entered sleep()
        // or permanently departed (ParticipantMockClock dropped).
        let guard = self.inner.settlement_state.lock().unwrap();
        let _guard = self
            .inner
            .settlement
            .wait_while(guard, |ss| {
                let departures = ss.departed - ss.departed_at_advance;
                ss.settled + departures < ss.expected
            })
            .unwrap();
    }
}

impl Clock for MockClock {
    fn now(&self) -> DateTime<Utc> {
        self.inner.state.lock().unwrap().time
    }

    fn sleep(&self, duration: Duration) {
        mock_sleep(&self.inner, duration);
    }

    fn for_thread(&self) -> Arc<dyn Clock> {
        Arc::new(ParticipantMockClock {
            clock: self.clone(),
        })
    }
}

/// Shared sleep implementation for `MockClock` and `ParticipantMockClock`.
fn mock_sleep(inner: &MockClockInner, duration: Duration) {
    let mut guard = inner.state.lock().unwrap();
    let deadline = guard.time + duration;
    if guard.time < deadline {
        // Register deadline so advance_and_settle() can count
        // how many threads will wake, and notify wait_for_sleepers.
        guard.deadlines.push(deadline);
        inner.condvar.notify_all();

        // Signal settlement: this thread has (re-)entered sleep.
        {
            let mut ss = inner.settlement_state.lock().unwrap();
            ss.settled += 1;
            inner.settlement.notify_all();
        }

        while guard.time < deadline {
            guard = inner.condvar.wait(guard).unwrap();
        }

        // Deregister deadline (we're leaving sleep).
        guard.deadlines.retain(|d| *d != deadline);
    }
}

/// A per-thread mock clock that signals departure from the settlement
/// protocol when dropped. Created by [`MockClock::for_thread()`].
///
/// When the thread holding this clock exits (normally or via panic),
/// the clock drops and increments the departure counter, so
/// `advance_and_settle()` won't block waiting for a dead thread to
/// re-enter `sleep()`.
struct ParticipantMockClock {
    clock: MockClock,
}

impl Clock for ParticipantMockClock {
    fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }

    fn sleep(&self, duration: Duration) {
        mock_sleep(&self.clock.inner, duration);
    }

    fn for_thread(&self) -> Arc<dyn Clock> {
        self.clock.for_thread()
    }
}

impl Drop for ParticipantMockClock {
    fn drop(&mut self) {
        let mut ss = self.clock.inner.settlement_state.lock().unwrap();
        ss.departed += 1;
        self.clock.inner.settlement.notify_all();
    }
}

#[cfg(test)]
mod tests;
