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

    clock.wait_for_waiters(1);
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

    let handle = std::thread::spawn(move || {
        clock2.sleep(Duration::from_secs(10));
        woke2.store(true, Ordering::SeqCst);
    });

    clock.wait_for_waiters(1);
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

    clock.wait_for_waiters(2);
    clock.advance(Duration::from_secs(5));

    let t1 = h1.join().unwrap();
    assert_eq!(t1, start + Duration::from_secs(5));

    // h2 should still be blocked; advance the remaining 5s.
    clock.wait_for_waiters(1);
    clock.advance(Duration::from_secs(5));

    let t2 = h2.join().unwrap();
    assert_eq!(t2, start + Duration::from_secs(10));
}
