use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use chrono::Utc;

use super::CaptureControl;
use crate::clock::{Clock, MockClock};

/// wait_while_paused returns false immediately when not paused.
#[test]
fn wait_while_paused_returns_false_when_running() {
    let control = CaptureControl::new();
    let clock = MockClock::new(Utc::now());
    let sync = clock.for_thread();
    assert!(!control.wait_while_paused(&*sync));
}

/// wait_while_paused returns true immediately when stopped.
#[test]
fn wait_while_paused_returns_true_when_stopped() {
    let control = CaptureControl::new();
    let clock = MockClock::new(Utc::now());
    let sync = clock.for_thread();
    control.stop();
    assert!(control.wait_while_paused(&*sync));
}

/// wait_while_paused blocks when paused; resume unblocks it
/// and returns false.
#[test]
fn pause_blocks_resume_unblocks() {
    let control = Arc::new(CaptureControl::new());
    let clock = MockClock::new(Utc::now());

    control.pause();

    let ctrl = Arc::clone(&control);
    let thread_clock = clock.for_thread();
    let returned_stopped = Arc::new(AtomicBool::new(false));
    let ret = Arc::clone(&returned_stopped);
    let handle = std::thread::spawn(move || {
        // Initial sleep: lets the harness track us.
        thread_clock.sleep(Duration::from_millis(100));
        // Enters wait_while_paused, sees paused, parks.
        let stopped = ctrl.wait_while_paused(&*thread_clock);
        ret.store(stopped, Ordering::Release);
        // Thread exits: participant clock departs.
    });

    // Wait for thread to enter sleep, then advance past it.
    // Thread wakes, enters wait_while_paused, parks (settled).
    clock.wait_for_sleepers(1);
    clock.advance_and_settle(Duration::from_millis(100));

    // Thread is now parked. Resume it.
    control.resume();
    handle.join().unwrap();

    assert!(
        !returned_stopped.load(Ordering::Acquire),
        "should return false on resume"
    );
}

/// stop while paused wakes threads; wait_while_paused returns true.
#[test]
fn stop_while_paused_returns_true() {
    let control = Arc::new(CaptureControl::new());
    let clock = MockClock::new(Utc::now());

    control.pause();

    let ctrl = Arc::clone(&control);
    let thread_clock = clock.for_thread();
    let returned_stopped = Arc::new(AtomicBool::new(false));
    let ret = Arc::clone(&returned_stopped);
    let handle = std::thread::spawn(move || {
        thread_clock.sleep(Duration::from_millis(100));
        let stopped = ctrl.wait_while_paused(&*thread_clock);
        ret.store(stopped, Ordering::Release);
    });

    clock.wait_for_sleepers(1);
    clock.advance_and_settle(Duration::from_millis(100));

    // Thread is parked. Stop wakes it.
    control.stop();
    handle.join().unwrap();

    assert!(returned_stopped.load(Ordering::Acquire));
}

/// resume sets clipboard_reseed; take_clipboard_reseed returns true
/// once, then false.
#[test]
fn resume_sets_clipboard_reseed_one_shot() {
    let control = CaptureControl::new();

    // Initially false.
    assert!(!control.take_clipboard_reseed());

    // Resume sets it.
    control.pause();
    control.resume();
    assert!(control.take_clipboard_reseed());

    // Second take returns false (one-shot).
    assert!(!control.take_clipboard_reseed());
}

/// Multiple pause/resume cycles: the thread runs one iteration per
/// cycle, blocking in wait_while_paused between cycles.
#[test]
fn multiple_pause_resume_cycles() {
    let control = Arc::new(CaptureControl::new());
    let clock = MockClock::new(Utc::now());
    let cycle_count = Arc::new(AtomicUsize::new(0));

    let ctrl = Arc::clone(&control);
    let thread_clock = clock.for_thread();
    let count = Arc::clone(&cycle_count);
    let handle = std::thread::spawn(move || {
        loop {
            if ctrl.wait_while_paused(&*thread_clock) {
                break;
            }
            thread_clock.sleep(Duration::from_millis(100));
            count.fetch_add(1, Ordering::Release);
        }
    });

    // Thread passes through wait_while_paused (not paused),
    // enters clock.sleep(100ms).
    clock.wait_for_sleepers(1);

    for i in 1..=3 {
        // Pause before advancing. The thread is currently in
        // sleep(100ms). When we advance and it wakes, it will
        // increment count, loop back, see paused, and park.
        control.pause();
        clock.advance_and_settle(Duration::from_millis(100));
        assert_eq!(cycle_count.load(Ordering::Acquire), i);

        // Thread is parked. Resume: it wakes from condvar, passes
        // through wait_while_paused, enters clock.sleep(100ms).
        control.resume();
        clock.wait_for_sleepers(1);
    }

    assert_eq!(cycle_count.load(Ordering::Acquire), 3);

    // Stop and advance so the thread wakes from sleep, loops to
    // wait_while_paused, sees stopped, and exits.
    control.stop();
    clock.advance_and_settle(Duration::from_millis(100));
    handle.join().unwrap();
}

/// Paused thread is settled via park guard: advance_and_settle
/// returns without waiting for the paused thread to re-enter
/// sleep, because the park guard has already marked it settled.
#[test]
fn parked_thread_settled_via_park_guard() {
    let clock = MockClock::new(Utc::now());
    let control = Arc::new(CaptureControl::new());

    // Spawn a thread that sleeps, then enters pause.
    let thread_clock = clock.for_thread();
    let ctrl = Arc::clone(&control);
    let _worker = std::thread::spawn(move || {
        // Initial sleep so the clock can track us.
        thread_clock.sleep(Duration::from_millis(100));
        // Enter pause: will park and block on condvar.
        ctrl.wait_while_paused(&*thread_clock);
    });

    // Wait for thread to enter initial sleep, then wake it.
    clock.wait_for_sleepers(1);
    control.pause();
    clock.advance_and_settle(Duration::from_millis(100));

    // Thread is now parked inside wait_while_paused. The park guard
    // has already settled it. advance_and_settle with 0 duration
    // should return immediately without blocking.
    clock.advance_and_settle(Duration::from_millis(0));

    // Clean up: stop so the thread exits.
    control.stop();
}

/// Resume from pause: thread wakes, guard drops (expected += 1),
/// then re-enters sleep (settled += 1). advance_and_settle waits
/// for the full cycle.
#[test]
fn resume_from_pause_settles_after_sleep() {
    let clock = MockClock::new(Utc::now());
    let control = Arc::new(CaptureControl::new());
    let work_done = Arc::new(AtomicBool::new(false));

    let thread_clock = clock.for_thread();
    let ctrl = Arc::clone(&control);
    let done = Arc::clone(&work_done);
    let _worker = std::thread::spawn(move || {
        loop {
            if ctrl.wait_while_paused(&*thread_clock) {
                break;
            }
            thread_clock.sleep(Duration::from_millis(100));
            done.store(true, Ordering::Release);
        }
    });

    // Thread starts running, enters sleep(100).
    clock.wait_for_sleepers(1);

    // Pause the thread: it will wake from sleep, call
    // wait_while_paused, and park on the condvar.
    control.pause();
    clock.advance_and_settle(Duration::from_millis(100));

    // Thread is now parked. Resume it.
    control.resume();

    // After resume, the park guard drops (expected += 1), and the
    // thread enters sleep(100) (settled += 1). We need to wait
    // for it to reach sleep.
    clock.wait_for_sleepers(1);

    // Now advance past its sleep deadline. Settlement should
    // complete: thread wakes, does work, re-enters sleep.
    clock.advance_and_settle(Duration::from_millis(100));
    assert!(work_done.load(Ordering::Acquire));

    control.stop();
}
