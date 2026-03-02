//! Injectable clock for deterministic time in tests.
//!
//! Two traits separate concerns:
//!
//! - [`Clock`]: timestamps and worker-clock factory. [`MockClock`]
//!   implements this but **not** `SyncClock`, so you can read time and
//!   create per-thread clocks but cannot accidentally sleep on a clock
//!   without departure tracking.
//!
//! - [`SyncClock`]: extends `Clock` with [`sleep()`](SyncClock::sleep)
//!   and [`park()`](SyncClock::park). Only [`RealClock`] and the
//!   internal `ParticipantMockClock` (returned by
//!   [`Clock::for_thread()`]) implement this.
//!
//! Production code uses [`RealClock`] (implements both traits). Test
//! mode substitutes a [`MockClock`] whose time only advances via
//! explicit [`advance()`](MockClock::advance) calls. Worker threads
//! obtain a [`SyncClock`] via [`Clock::for_thread()`], which returns a
//! `ParticipantMockClock` that signals departure on drop — so
//! [`advance_and_settle()`](MockClock::advance_and_settle) never blocks
//! waiting for a dead thread.

use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use chrono::{DateTime, Utc};

/// Timestamps and worker-clock factory.
///
/// Does not participate in the settlement protocol — calling code
/// cannot sleep or park through this trait. To sleep, obtain a
/// [`SyncClock`] via [`for_thread()`](Clock::for_thread).
pub trait Clock: Send + Sync {
    /// Current wall-clock time (UTC).
    fn now(&self) -> DateTime<Utc>;

    /// Create a per-thread clock that participates in settlement.
    ///
    /// The returned clock signals departure when dropped, so
    /// `advance_and_settle()` won't block waiting for a thread that has
    /// exited. Production clocks return a plain `RealClock` (no-op drop).
    fn for_thread(&self) -> Arc<dyn SyncClock>;
}

/// Settlement-participating clock: sleep, park, plus [`Clock`] supertrait.
///
/// Only [`RealClock`] and the internal `ParticipantMockClock` implement
/// this. [`MockClock`] deliberately does not — you must call
/// [`Clock::for_thread()`] to obtain a `SyncClock` with proper departure
/// tracking.
pub trait SyncClock: Clock {
    /// Sleep for the given duration.
    ///
    /// Production: real `thread::sleep()`.
    /// Test mode: blocks on a condvar until `advance()` meets the deadline.
    fn sleep(&self, duration: Duration);

    /// Signal that this thread is about to block on an external primitive.
    ///
    /// Returns a guard that re-enters settlement tracking when dropped.
    /// Use this before blocking on a condvar, channel, barrier, or any
    /// synchronization primitive that the clock can't see.
    ///
    /// Production: returns a no-op guard.
    /// Test mode: increments `settled` on creation (satisfying
    /// `advance_and_settle`), increments `expected` on drop (requiring
    /// the thread to reach `sleep()` before the next settlement).
    fn park(&self) -> ParkGuard {
        ParkGuard { inner: None }
    }
}

/// Guard returned by [`SyncClock::park()`].
///
/// Brackets an external block (condvar wait, channel recv, etc.) so the
/// settlement protocol can track threads that are blocked outside the
/// clock's view. On creation, `settled += 1` (this thread has done its
/// tick work). On drop, `expected += 1` (this thread needs to reach
/// `sleep()` before quiescence).
pub struct ParkGuard {
    inner: Option<Arc<MockClockInner>>,
}

impl Drop for ParkGuard {
    fn drop(&mut self) {
        if let Some(ref inner) = self.inner {
            let mut ss = inner.settlement_state.lock().unwrap();
            ss.expected += 1;
            inner.settlement.notify_all();
        }
    }
}

/// Spawn a thread that participates in the clock's settlement protocol.
///
/// The thread receives a per-thread [`SyncClock`] via
/// [`Clock::for_thread()`]. When the thread exits, its clock drops and
/// departure is signaled, so `advance_and_settle()` won't block waiting
/// for a dead thread.
pub fn spawn_clock_thread<F, T>(name: &str, clock: &dyn Clock, f: F) -> JoinHandle<T>
where
    F: FnOnce(Arc<dyn SyncClock>) -> T + Send + 'static,
    T: Send + 'static,
{
    let thread_clock = clock.for_thread();
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || f(thread_clock))
        .expect("failed to spawn clock thread")
}

/// Production clock: real wall-clock time and real sleep.
#[derive(Debug, Clone, Copy)]
pub struct RealClock;

impl Clock for RealClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn for_thread(&self) -> Arc<dyn SyncClock> {
        Arc::new(RealClock)
    }
}

impl SyncClock for RealClock {
    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// Test clock: time only advances via explicit `advance()` calls.
///
/// Implements [`Clock`] (timestamps + factory) but **not** [`SyncClock`].
/// To sleep, call [`for_thread()`](Clock::for_thread) to obtain a
/// `ParticipantMockClock` that participates in the settlement protocol
/// with proper departure tracking.
///
/// `advance()` bumps the internal time and broadcasts a condvar.
/// Sleeping threads (on `ParticipantMockClock`) whose deadlines are met
/// wake and return. This eliminates both CPU spin and real wall-clock
/// delay: threads proceed in lockstep with harness-driven time.
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

    fn for_thread(&self) -> Arc<dyn SyncClock> {
        Arc::new(ParticipantMockClock {
            clock: self.clone(),
        })
    }
}

/// Shared sleep implementation for `ParticipantMockClock`.
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

    fn for_thread(&self) -> Arc<dyn SyncClock> {
        self.clock.for_thread()
    }
}

impl SyncClock for ParticipantMockClock {
    fn sleep(&self, duration: Duration) {
        mock_sleep(&self.clock.inner, duration);
    }

    fn park(&self) -> ParkGuard {
        let mut ss = self.clock.inner.settlement_state.lock().unwrap();
        ss.settled += 1;
        self.clock.inner.settlement.notify_all();
        drop(ss);
        ParkGuard {
            inner: Some(Arc::clone(&self.clock.inner)),
        }
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
