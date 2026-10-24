//! The hub's timers: re-ping, snooze end, expiry, waiter lapse and the daily
//! retention sweep, kept as wall-clock deadlines.
//!
//! The actor fires what is due before every command and whenever it wakes
//! from idle. A fired timer only says "look again": the actor re-reads the
//! message and re-runs the policy, so a message answered or rescinded in the
//! meantime sends nothing (architecture §6.1).

use std::collections::BTreeSet;

use gentbox_core::ids::MessageId;
use time::OffsetDateTime;

/// What a timer is for.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum TimerKind {
    /// A still-open message may ping again.
    Reping(MessageId),
    /// A snoozed message returns to `open`.
    SnoozeEnd(MessageId),
    /// A live message expires.
    Expiry(MessageId),
    /// A message's live waiter may have lapsed.
    WaiterLapse(MessageId),
    /// The retention sweep.
    Sweep,
}

/// Deadlines, earliest first. The same timer may be scheduled twice; firing is
/// idempotent because the actor checks the message's state.
#[derive(Debug, Default)]
pub(crate) struct Timers {
    deadlines: BTreeSet<(OffsetDateTime, TimerKind)>,
}

impl Timers {
    /// Schedules `kind` at `at`.
    pub(crate) fn schedule(&mut self, at: OffsetDateTime, kind: TimerKind) {
        self.deadlines.insert((at, kind));
    }

    /// The earliest deadline, if any.
    pub(crate) fn next_deadline(&self) -> Option<OffsetDateTime> {
        self.deadlines.first().map(|(at, _)| *at)
    }

    /// Removes and returns every timer due at `now`, earliest first.
    pub(crate) fn take_due(&mut self, now: OffsetDateTime) -> Vec<TimerKind> {
        let mut due = Vec::new();
        while let Some((at, _)) = self.deadlines.first() {
            if *at > now {
                break;
            }
            if let Some((_, kind)) = self.deadlines.pop_first() {
                due.push(kind);
            }
        }
        due
    }

    /// Drops the timers of a message that closed. Its waiter-lapse timers
    /// stay: when they fire, the actor forgets the message's waiter state.
    pub(crate) fn cancel_message(&mut self, id: &MessageId) {
        self.deadlines.retain(|(_, kind)| match kind {
            TimerKind::Reping(of) | TimerKind::SnoozeEnd(of) | TimerKind::Expiry(of) => of != id,
            TimerKind::WaiterLapse(_) | TimerKind::Sweep => true,
        });
    }

    /// Drops a message's re-ping timers (it was acknowledged or deferred).
    pub(crate) fn cancel_repings(&mut self, id: &MessageId) {
        self.deadlines
            .retain(|(_, kind)| !matches!(kind, TimerKind::Reping(of) if of == id));
    }
}

#[cfg(test)]
mod tests {
    use time::Duration;
    use time::macros::datetime;

    use super::*;

    #[test]
    fn due_timers_come_out_earliest_first_and_later_ones_stay() {
        let now = datetime!(2026-10-24 12:00 UTC);
        let mut timers = Timers::default();
        let one = MessageId::new("one");
        timers.schedule(now + Duration::seconds(5), TimerKind::Expiry(one.clone()));
        timers.schedule(now, TimerKind::Reping(one.clone()));
        timers.schedule(now + Duration::minutes(5), TimerKind::Sweep);
        assert_eq!(
            timers.take_due(now + Duration::seconds(5)),
            [TimerKind::Reping(one.clone()), TimerKind::Expiry(one)]
        );
        assert_eq!(timers.next_deadline(), Some(now + Duration::minutes(5)));
    }

    #[test]
    fn cancelling_a_message_keeps_other_timers() {
        let now = datetime!(2026-10-24 12:00 UTC);
        let mut timers = Timers::default();
        let (one, two) = (MessageId::new("one"), MessageId::new("two"));
        timers.schedule(now, TimerKind::Reping(one.clone()));
        timers.schedule(now, TimerKind::WaiterLapse(one.clone()));
        timers.schedule(now, TimerKind::Expiry(two.clone()));
        timers.schedule(now, TimerKind::Sweep);
        timers.cancel_message(&one);
        assert_eq!(
            timers.take_due(now),
            [
                TimerKind::Expiry(two),
                TimerKind::WaiterLapse(one),
                TimerKind::Sweep
            ]
        );
    }
}
