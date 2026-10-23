//! The retention sweep (architecture §5.2, ADRs 0008 and 0009).
//!
//! The hub runs it at start and daily, passing `now`.

use gentbox_core::lifecycle::LifecycleState;
use rusqlite::params;
use time::{Duration, OffsetDateTime};

use crate::codec::to_nanos;
use crate::error::StoreError;
use crate::repository::Repository;

/// How long closed records are kept: 30 days, as architecture §5.2 says.
pub const DEFAULT_RETENTION: Duration = Duration::days(30);

/// How long the sweep keeps what it may delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// Answered, dismissed, rescinded and expired messages are deleted this
    /// long after they closed.
    pub message_retention: Duration,
    /// Closed or implicit topics and threads with no messages left are deleted
    /// this long after they closed (or, for an open implicit one, after their
    /// last change). The architecture says "the message retention period", so
    /// this defaults to the same value.
    pub group_retention: Duration,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            message_retention: DEFAULT_RETENTION,
            group_retention: DEFAULT_RETENTION,
        }
    }
}

/// What a sweep deleted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// Messages, with their actions, answers and delivery log.
    pub messages: u32,
    /// Threads.
    pub threads: u32,
    /// Topics.
    pub topics: u32,
    /// Conversations that no message and no default thread refers to any more.
    pub conversations: u32,
}

impl Repository<'_> {
    /// Deletes what retention allows, in one transaction:
    ///
    /// 1. **Messages** that are answered, dismissed, rescinded or expired and
    ///    closed before `now - message_retention`. Open and acknowledged
    ///    messages are never deleted, however old. A message that replaced
    ///    another one stays referenced: deleting the replaced message
    ///    clears nothing else, deleting the successor clears the link.
    /// 2. **Threads** with no messages that are closed, or implicit, and whose
    ///    last change (closing, for a closed one) is before
    ///    `now - group_retention`. A thread you muted is never deleted, so the
    ///    mute survives.
    /// 3. **Topics** under the same rules that also have no threads left. A
    ///    topic you muted is never deleted, and neither is an agent's General
    ///    topic.
    /// 4. **Conversations** no message and no default thread refers to, last
    ///    seen before `now - message_retention`.
    ///
    /// A topic or thread that still has messages is never deleted.
    pub fn sweep(
        &self,
        now: OffsetDateTime,
        policy: &RetentionPolicy,
    ) -> Result<SweepReport, StoreError> {
        let message_cutoff = to_nanos(now - policy.message_retention)?;
        let group_cutoff = to_nanos(now - policy.group_retention)?;
        self.atomically(|| {
            let closed_states = closed_state_list();
            let messages = self
                .connection
                .prepare_cached(&format!(
                    "DELETE FROM messages WHERE state IN ({closed_states}) \
                 AND closed_at IS NOT NULL AND closed_at < ?1"
                ))?
                .execute(params![message_cutoff])?;
            let threads = self
                .connection
                .prepare_cached(
                    "DELETE FROM threads WHERE (status = 'closed' OR implicit = 1) \
                 AND muted = 0 \
                 AND COALESCE(closed_at, updated_at) < ?1 \
                 AND NOT EXISTS (SELECT 1 FROM messages WHERE messages.thread_id = threads.id)",
                )?
                .execute(params![group_cutoff])?;
            let topics = self
                .connection
                .prepare_cached(
                    "DELETE FROM topics WHERE (status = 'closed' OR implicit = 1) \
                 AND muted = 0 AND NOT (implicit = 1 AND kind = 'general') \
                 AND COALESCE(closed_at, updated_at) < ?1 \
                 AND NOT EXISTS (SELECT 1 FROM messages WHERE messages.topic_id = topics.id) \
                 AND NOT EXISTS (SELECT 1 FROM threads WHERE threads.topic_id = topics.id)",
                )?
                .execute(params![group_cutoff])?;
            let conversations = self
                .connection
                .prepare_cached(
                    "DELETE FROM conversations WHERE updated_at < ?1 \
                 AND NOT EXISTS (SELECT 1 FROM messages \
                     WHERE messages.source_agent = conversations.agent \
                     AND messages.conversation_id = conversations.conversation_id) \
                 AND NOT EXISTS (SELECT 1 FROM conversation_default_threads d \
                     WHERE d.agent = conversations.agent \
                     AND d.conversation_id = conversations.conversation_id)",
                )?
                .execute(params![message_cutoff])?;
            Ok(SweepReport {
                messages: count_of(messages)?,
                threads: count_of(threads)?,
                topics: count_of(topics)?,
                conversations: count_of(conversations)?,
            })
        })
    }
}

fn closed_state_list() -> String {
    [
        LifecycleState::Answered,
        LifecycleState::Dismissed,
        LifecycleState::Rescinded,
        LifecycleState::Expired,
    ]
    .iter()
    .map(|state| format!("'{}'", crate::codec::enum_text(state)))
    .collect::<Vec<_>>()
    .join(", ")
}

fn count_of(rows: usize) -> Result<u32, StoreError> {
    u32::try_from(rows).map_err(|_| StoreError::Constraint("too many rows deleted".to_owned()))
}
