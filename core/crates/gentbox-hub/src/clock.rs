//! The hub's clock. Wall-clock time is a dependency like any other IO, so
//! tests can drive timers (re-ping, snooze, expiry, waiter lapse) without
//! waiting for them.

use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use time::{Duration, OffsetDateTime};

/// Where the hub reads "now".
pub trait Clock: Send + Sync + Debug {
    /// The current instant.
    fn now(&self) -> OffsetDateTime;
}

/// The system clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}

/// A clock that moves only when told to, for tests. Clones share one time.
#[derive(Debug, Clone)]
pub struct ManualClock {
    now: Arc<Mutex<OffsetDateTime>>,
}

impl ManualClock {
    /// A clock stopped at `start`.
    pub fn new(start: OffsetDateTime) -> Self {
        Self {
            now: Arc::new(Mutex::new(start)),
        }
    }

    /// Moves the clock forward by `step`. Call
    /// [`crate::HubHandle::run_due_timers`] afterwards to fire what fell due.
    pub fn advance(&self, step: Duration) {
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        *now += step;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> OffsetDateTime {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
