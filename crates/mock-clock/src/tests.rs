use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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

/// Multiple advance calls accumulate.
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
    let sleeper = clock.for_thread();
    sleeper.sleep(Duration::ZERO);
    assert_eq!(clock.now(), start);
}

/// sleep() blocks until advance meets the deadline, and does not
/// itself move time forward.
#[test]
fn mock_sleep_blocks_until_deadline() {
    let start = Utc::now();
    let clock = MockClock::new(start);
    let sleeper = clock.for_thread();

    let handle = std::thread::spawn(move || {
        sleeper.sleep(Duration::from_secs(10));
        sleeper.now()
    });

    clock.wait_for_sleepers(1);
    clock.advance(Duration::from_secs(10));

    let woke_at = handle.join().unwrap();
    assert_eq!(woke_at, start + Duration::from_secs(10));
}

/// A partial advance doesn't wake a thread whose deadline isn't met.
#[test]
fn mock_sleep_partial_advance_stays_blocked() {
    let start = Utc::now();
    let clock = MockClock::new(start);
    let sleeper = clock.for_thread();
    let woke = Arc::new(AtomicBool::new(false));
    let woke2 = Arc::clone(&woke);

    let handle = std::thread::spawn(move || {
        sleeper.sleep(Duration::from_secs(10));
        woke2.store(true, Ordering::SeqCst);
    });

    clock.wait_for_sleepers(1);
    clock.advance(Duration::from_secs(5));

    // Thread needs 10s but only 5s have passed — still blocked.
    assert!(!woke.load(Ordering::SeqCst));

    // Remaining 5s meets the deadline.
    clock.advance(Duration::from_secs(5));
    handle.join().unwrap();
    assert!(woke.load(Ordering::SeqCst));
}

/// Multiple threads sleeping with different deadlines wake independently.
#[test]
fn mock_sleep_multiple_threads_different_deadlines() {
    let start = Utc::now();
    let clock = MockClock::new(start);

    let c1 = clock.for_thread();
    let c2 = clock.for_thread();

    let h1 = std::thread::spawn(move || {
        c1.sleep(Duration::from_secs(5));
        c1.now()
    });
    let h2 = std::thread::spawn(move || {
        c2.sleep(Duration::from_secs(10));
        c2.now()
    });

    clock.wait_for_sleepers(2);
    clock.advance(Duration::from_secs(5));

    let t1 = h1.join().unwrap();
    assert_eq!(t1, start + Duration::from_secs(5));

    // h2 should still be blocked; advance the remaining 5s.
    clock.wait_for_sleepers(1);
    clock.advance(Duration::from_secs(5));

    let t2 = h2.join().unwrap();
    assert_eq!(t2, start + Duration::from_secs(10));
}

/// ACK protocol invariant: advance_and_settle returns only after woken
/// threads complete work and re-enter sleep.
#[test]
fn settlement_waits_for_resleep_after_work() {
    let start = Utc::now();
    let clock = MockClock::new(start);
    let work_done = Arc::new(AtomicBool::new(false));

    let c = clock.for_thread();
    let done = Arc::clone(&work_done);
    let _worker = std::thread::spawn(move || {
        loop {
            c.sleep(Duration::from_millis(100));
            std::thread::yield_now();
            done.store(true, Ordering::Release);
        }
    });

    clock.wait_for_sleepers(1);
    clock.advance_and_settle(Duration::from_millis(100));

    assert!(work_done.load(Ordering::Acquire));
}

/// advance_and_settle with no sleeping threads returns immediately.
#[test]
fn settlement_no_sleepers_returns_immediately() {
    let clock = MockClock::new(Utc::now());
    clock.advance_and_settle(Duration::from_millis(100));
}

/// Multiple wake-work-resleep cycles: advance_and_settle returns
/// correctly on each cycle without accumulation bugs.
#[test]
fn settlement_multiple_cycles() {
    let start = Utc::now();
    let clock = MockClock::new(start);
    let cycle_count = Arc::new(AtomicUsize::new(0));

    let c = clock.for_thread();
    let count = Arc::clone(&cycle_count);
    let _worker = std::thread::spawn(move || {
        loop {
            c.sleep(Duration::from_millis(50));
            count.fetch_add(1, Ordering::Release);
        }
    });

    clock.wait_for_sleepers(1);

    for i in 1..=10 {
        clock.advance_and_settle(Duration::from_millis(50));
        assert_eq!(cycle_count.load(Ordering::Acquire), i);
    }
}

/// Settlement completes when a woken thread exits instead of re-sleeping,
/// provided it uses a participant clock (via `for_thread()`).
#[test]
fn settlement_completes_on_participant_departure() {
    let clock = MockClock::new(Utc::now());

    let thread_clock = clock.for_thread();
    let _worker = std::thread::spawn(move || {
        thread_clock.sleep(Duration::from_millis(100));
        // Thread exits without re-sleeping. The ParticipantMockClock
        // drops, signaling departure so settlement doesn't block.
    });

    clock.wait_for_sleepers(1);
    clock.advance_and_settle(Duration::from_millis(100));
    // If we get here, settlement correctly handled the departure.
}

/// Settlement handles a mix of threads that re-sleep and threads that depart.
#[test]
fn settlement_mixed_resleep_and_departure() {
    let clock = MockClock::new(Utc::now());
    let work_done = Arc::new(AtomicBool::new(false));

    // Thread A: loops and re-sleeps (stays alive).
    let stayer_clock = clock.for_thread();
    let done = Arc::clone(&work_done);
    let _stayer = std::thread::spawn(move || {
        loop {
            stayer_clock.sleep(Duration::from_millis(100));
            done.store(true, Ordering::Release);
        }
    });

    // Thread B: sleeps once, then exits.
    let leaver_clock = clock.for_thread();
    let _leaver = std::thread::spawn(move || {
        leaver_clock.sleep(Duration::from_millis(100));
        // Exits — participant clock drops.
    });

    clock.wait_for_sleepers(2);
    clock.advance_and_settle(Duration::from_millis(100));

    // Thread A settled by re-sleeping, Thread B settled by departing.
    assert!(work_done.load(Ordering::Acquire));
}

// --- ParkGuard / SyncClock::park() tests ---

/// Park → external block → resume → guard drops → sleep: settlement
/// waits for the full cycle.
///
/// Simulates a thread that parks (enters an external condvar wait),
/// wakes from the external block, drops its guard, then re-enters
/// clock.sleep(). advance_and_settle must not return until the thread
/// reaches sleep.
#[test]
fn park_resume_then_sleep_settles() {
    let clock = MockClock::new(Utc::now());
    let external_condvar = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let work_done = Arc::new(AtomicBool::new(false));

    let thread_clock = clock.for_thread();
    let ext = Arc::clone(&external_condvar);
    let done = Arc::clone(&work_done);
    let _worker = std::thread::spawn(move || {
        // Initial sleep to register with the clock.
        thread_clock.sleep(Duration::from_millis(100));

        // Park: settled += 1, then block on external condvar.
        {
            let _guard = thread_clock.park();
            let (lock, cvar) = &*ext;
            let mut ready = lock.lock().unwrap();
            while !*ready {
                ready = cvar.wait(ready).unwrap();
            }
            // _guard drops here: expected += 1
        }

        // Do work after waking from external block.
        done.store(true, Ordering::Release);

        // Re-enter clock sleep: settled += 1, balancing the expected.
        thread_clock.sleep(Duration::from_millis(100));
    });

    // Wait for thread to enter initial sleep, then advance past it.
    clock.wait_for_sleepers(1);
    clock.advance_and_settle(Duration::from_millis(100));

    // Thread is now parked on the external condvar. advance_and_settle
    // with zero duration should return immediately (thread is settled
    // via park).
    clock.advance_and_settle(Duration::from_millis(0));

    // Wake the external condvar. The thread's guard will drop
    // (expected += 1), do work, then re-enter sleep (settled += 1).
    {
        let (lock, cvar) = &*external_condvar;
        *lock.lock().unwrap() = true;
        cvar.notify_all();
    }

    // The thread needs to reach sleep. advance_and_settle should
    // wait for it. We advance just enough to meet the new deadline.
    clock.wait_for_sleepers(1);
    clock.advance_and_settle(Duration::from_millis(100));

    assert!(work_done.load(Ordering::Acquire));
}

/// Park → stop → guard drops → thread exits: settlement completes
/// via departure when a parked thread is told to stop.
#[test]
fn park_then_exit_settles_via_departure() {
    let clock = MockClock::new(Utc::now());
    let external_condvar = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));

    let thread_clock = clock.for_thread();
    let ext = Arc::clone(&external_condvar);
    let worker = std::thread::spawn(move || {
        // Initial sleep to register with the clock.
        thread_clock.sleep(Duration::from_millis(100));

        // Park and block on external condvar.
        {
            let _guard = thread_clock.park();
            let (lock, cvar) = &*ext;
            let mut ready = lock.lock().unwrap();
            while !*ready {
                ready = cvar.wait(ready).unwrap();
            }
            // _guard drops: expected += 1
        }

        // Thread exits without re-sleeping.
        // ParticipantMockClock drops: departed += 1
    });

    clock.wait_for_sleepers(1);
    clock.advance_and_settle(Duration::from_millis(100));

    // Thread is parked. Wake it and let it exit.
    {
        let (lock, cvar) = &*external_condvar;
        *lock.lock().unwrap() = true;
        cvar.notify_all();
    }

    worker.join().unwrap();

    // Settlement should handle the guard drop (expected += 1) and
    // departure (departed += 1) without blocking.
    clock.advance_and_settle(Duration::from_millis(0));
}

/// Park on RealClock returns a no-op guard: drop doesn't panic.
#[test]
fn real_clock_park_is_noop() {
    let clock = RealClock;
    let guard = clock.park();
    drop(guard);
}

/// Multiple park/drop cycles in a loop: each cycle is balanced
/// (settled +1 from park, expected +1 from drop), and the final
/// sleep settles the thread.
#[test]
fn park_multiple_cycles_settles() {
    let clock = MockClock::new(Utc::now());
    let gate = Arc::new((std::sync::Mutex::new(0u32), std::sync::Condvar::new()));
    let cycles_completed = Arc::new(AtomicUsize::new(0));

    let thread_clock = clock.for_thread();
    let g = Arc::clone(&gate);
    let completed = Arc::clone(&cycles_completed);
    let _worker = std::thread::spawn(move || {
        thread_clock.sleep(Duration::from_millis(100));

        // Three park/drop cycles, simulating spurious wakes from a
        // condvar-based pause loop.
        for i in 1..=3 {
            {
                let _guard = thread_clock.park();
                let (lock, cvar) = &*g;
                let mut val = lock.lock().unwrap();
                while *val < i {
                    val = cvar.wait(val).unwrap();
                }
            }
            completed.fetch_add(1, Ordering::Release);
        }

        // Final sleep to settle.
        thread_clock.sleep(Duration::from_millis(100));
    });

    clock.wait_for_sleepers(1);
    clock.advance_and_settle(Duration::from_millis(100));

    // Release each cycle one at a time.
    for i in 1..=3 {
        {
            let (lock, cvar) = &*gate;
            *lock.lock().unwrap() = i;
            cvar.notify_all();
        }
        // After releasing, the thread's guard drops (expected += 1),
        // it increments the counter, then either parks again or sleeps.
        // We need to wait for it to reach either park or sleep.
        if i < 3 {
            // Thread will re-park: settlement sees it as settled.
            // A zero-duration advance should settle immediately once
            // the thread re-parks.
            // Give the thread a moment to reach the next park().
            std::thread::yield_now();
        }
    }

    // After all 3 cycles, thread enters final sleep.
    clock.wait_for_sleepers(1);
    clock.advance_and_settle(Duration::from_millis(100));
    assert_eq!(cycles_completed.load(Ordering::Acquire), 3);
}
