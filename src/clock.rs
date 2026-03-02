//! Clock re-exports and process-wide factory.
//!
//! All clock types live in the `attend-mock-clock` crate. This module
//! re-exports them for convenience and provides [`process_clock()`],
//! which selects MockClock or RealClock based on test mode.

pub use attend_mock_clock::*;

/// Create the process-wide clock.
///
/// Returns the `MockClock` from [`crate::test_mode`] if test mode is
/// active, otherwise a [`RealClock`].
pub fn process_clock() -> std::sync::Arc<dyn Clock> {
    if let Some(clock) = crate::test_mode::clock() {
        clock
    } else {
        std::sync::Arc::new(RealClock)
    }
}
