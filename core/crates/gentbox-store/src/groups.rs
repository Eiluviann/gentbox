//! Topics, threads and conversations' default threads (ADRs 0008, 0009), as
//! methods of [`Repository`].
//!
//! The store keeps the records and the multi-row operations that must be
//! atomic: takeover, moving a thread, closing and reopening, marking read. The
//! decisions (which topic, which thread, whether a takeover applies) are
//! `gentbox_core::resolve`'s; the hub passes the outcome in.

use gentbox_core::ids::{MessageId, ThreadId, TopicId};
use gentbox_core::resolve::{DefaultThreadLink, GroupingSnapshot, Takeover};
use gentbox_core::topic::{
    CreatedBy, DefaultFor, GroupStatus, Thread, ThreadRecord, Topic, TopicRecord, TopicStyle,
};
use rusqlite::types::Value;
use rusqlite::{OptionalExtension, Row, params, params_from_iter};
use time::OffsetDateTime;

use crate::codec::{
    enum_text, from_nanos, from_optional_json, from_optional_nanos, parse_enum, to_nanos,
    to_optional_json, to_optional_nanos,
};
use crate::error::{Conflict, EntityKind, StoreError};
use crate::repository::Repository;

/// The result of an upsert by key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Upserted<T> {
    /// The key was new: the record was stored (201).
    Created(T),
    /// The key existed: the stored record, unchanged (200).
    Existing(T),
}

impl<T> Upserted<T> {
    /// The record, whichever way it came.
    pub fn into_inner(self) -> T {
        match self {
            Self::Created(value) | Self::Existing(value) => value,
        }
    }

    /// Whether the record was just created.
    pub fn was_created(&self) -> bool {
        matches!(self, Self::Created(_))
    }
}

/// What reopening a thread reopened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadReopened {
    /// The thread was closed and is open now.
    pub thread: bool,
    /// Its topic was closed too and is open now. Sibling threads that the
    /// topic's closing had closed stay closed.
    pub topic: bool,
}

impl Repository<'_> {
    // ---------------------------------------------------------------------
    // Topics
    // ---------------------------------------------------------------------

    /// Stores a new topic.
    ///
    /// # Errors
    ///
    /// [`Conflict::TopicId`], [`Conflict::TopicKey`], and
    /// [`Conflict::GeneralTopic`] when the agent already has a General topic.
    pub fn insert_topic(&self, topic: &Topic) -> Result<(), StoreError> {
        self.atomically(|| {
            if self.exists(
                "SELECT 1 FROM topics WHERE id = ?1",
                params![topic.id.as_str()],
            )? {
                return Err(Conflict::TopicId(topic.id.clone()).into());
            }
            if let Some(key) = &topic.key
                && self.exists("SELECT 1 FROM topics WHERE key = ?1", params![key])?
            {
                return Err(Conflict::TopicKey(key.clone()).into());
            }
            if topic.is_general_topic_of(&topic.created_by.agent)
                && self.exists(
                    "SELECT 1 FROM topics WHERE implicit = 1 AND kind = 'general' \
                     AND created_by_agent = ?1",
                    params![topic.created_by.agent],
                )?
            {
                return Err(Conflict::GeneralTopic(topic.created_by.agent.clone()).into());
            }
            self.connection
                .prepare_cached(
                    "INSERT INTO topics (id, key, title, summary, kind, style, implicit, status, \
                 muted, marked_read_at, created_by_agent, created_by_conversation_id, \
                 created_at, updated_at, closed_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                )?
                .execute(params![
                    topic.id.as_str(),
                    topic.key,
                    topic.title,
                    topic.summary,
                    topic.kind,
                    to_optional_json("style", topic.style.as_ref())?,
                    topic.implicit,
                    enum_text(&topic.status),
                    topic.muted,
                    to_optional_nanos(topic.marked_read_at)?,
                    topic.created_by.agent,
                    topic.created_by.conversation_id,
                    to_nanos(topic.created_at)?,
                    to_nanos(topic.updated_at)?,
                    to_optional_nanos(topic.closed_at)?,
                ])?;
            Ok(())
        })
    }

    /// `POST /v1/topics`: stores `topic` unless its key exists, in which case
    /// the stored topic comes back unchanged.
    ///
    /// # Panics
    ///
    /// When `topic` has no key: only keyed topics upsert by key.
    pub fn upsert_topic(&self, topic: &Topic) -> Result<Upserted<Topic>, StoreError> {
        let Some(key) = topic.key.as_deref() else {
            panic!("a topic upserted by key needs a key");
        };
        self.atomically(|| match self.find_topic_by_key(key)? {
            Some(existing) => Ok(Upserted::Existing(existing)),
            None => {
                self.insert_topic(topic)?;
                Ok(Upserted::Created(self.topic(&topic.id)?))
            }
        })
    }

    /// One topic.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] when there is none.
    pub fn topic(&self, id: &TopicId) -> Result<Topic, StoreError> {
        self.select_topics(" WHERE id = ?1", vec![Value::Text(id.to_string())])?
            .pop()
            .ok_or_else(|| StoreError::not_found(EntityKind::Topic, id))
    }

    /// The topic with this key, if any.
    pub fn find_topic_by_key(&self, key: &str) -> Result<Option<Topic>, StoreError> {
        Ok(self
            .select_topics(" WHERE key = ?1", vec![Value::Text(key.to_owned())])?
            .pop())
    }

    /// `agent`'s implicit General topic, if it has been created.
    pub fn find_general_topic(&self, agent: &str) -> Result<Option<Topic>, StoreError> {
        Ok(self
            .select_topics(
                " WHERE implicit = 1 AND kind = 'general' AND created_by_agent = ?1",
                vec![Value::Text(agent.to_owned())],
            )?
            .pop())
    }

    /// Topics, oldest first, optionally only open or only closed ones.
    pub fn list_topics(&self, status: Option<GroupStatus>) -> Result<Vec<Topic>, StoreError> {
        match status {
            None => self.select_topics(" ORDER BY created_at, id", Vec::new()),
            Some(status) => self.select_topics(
                " WHERE status = ?1 ORDER BY created_at, id",
                vec![Value::Text(enum_text(&status))],
            ),
        }
    }

    /// Saves what an agent or you may change on a topic: title, summary, kind,
    /// style, status, mute, mark-read time and the timestamps. Identity, key,
    /// `implicit` and `created_by` never change.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] when the topic is not stored.
    pub fn update_topic(&self, topic: &Topic) -> Result<(), StoreError> {
        let changed = self
            .connection
            .prepare_cached(
                "UPDATE topics SET title = ?2, summary = ?3, kind = ?4, style = ?5, status = ?6, \
             muted = ?7, marked_read_at = ?8, updated_at = ?9, closed_at = ?10 WHERE id = ?1",
            )?
            .execute(params![
                topic.id.as_str(),
                topic.title,
                topic.summary,
                topic.kind,
                to_optional_json("style", topic.style.as_ref())?,
                enum_text(&topic.status),
                topic.muted,
                to_optional_nanos(topic.marked_read_at)?,
                to_nanos(topic.updated_at)?,
                to_optional_nanos(topic.closed_at)?,
            ])?;
        if changed == 0 {
            return Err(StoreError::not_found(EntityKind::Topic, &topic.id));
        }
        Ok(())
    }

    /// A topic as the hub returns it, with its live counts.
    pub fn topic_record(&self, id: &TopicId) -> Result<TopicRecord, StoreError> {
        let topic = self.topic(id)?;
        self.record_of_topic(&topic)
    }

    /// Topic records, oldest first, optionally filtered by status.
    pub fn list_topic_records(
        &self,
        status: Option<GroupStatus>,
    ) -> Result<Vec<TopicRecord>, StoreError> {
        self.list_topics(status)?
            .iter()
            .map(|topic| self.record_of_topic(topic))
            .collect()
    }

    /// Closes a topic and the open threads in it (they remember that the
    /// topic closed them). Open messages stay. Closing a closed topic changes
    /// nothing. Returns the threads this call closed.
    pub fn close_topic(
        &self,
        id: &TopicId,
        now: OffsetDateTime,
    ) -> Result<Vec<ThreadId>, StoreError> {
        self.atomically(|| {
            let topic = self.topic(id)?;
            if topic.status == GroupStatus::Closed {
                return Ok(Vec::new());
            }
            let instant = to_nanos(now)?;
            self.connection
                .prepare_cached(
                    "UPDATE topics SET status = 'closed', closed_at = ?2, updated_at = ?2 \
                 WHERE id = ?1",
                )?
                .execute(params![id.as_str(), instant])?;
            let closed = self.thread_ids(
                "SELECT id FROM threads WHERE topic_id = ?1 AND status = 'open' ORDER BY id",
                id.as_str(),
            )?;
            self.connection
                .prepare_cached(
                    "UPDATE threads SET status = 'closed', closed_by_topic = 1, closed_at = ?2, \
                 updated_at = ?2 WHERE topic_id = ?1 AND status = 'open'",
                )?
                .execute(params![id.as_str(), instant])?;
            Ok(closed)
        })
    }

    /// Reopens a topic and only the threads its closing had closed; threads
    /// closed on their own stay closed. Reopening an open topic changes
    /// nothing. Returns the threads this call reopened.
    pub fn reopen_topic(
        &self,
        id: &TopicId,
        now: OffsetDateTime,
    ) -> Result<Vec<ThreadId>, StoreError> {
        self.atomically(|| {
            let topic = self.topic(id)?;
            if topic.status == GroupStatus::Open {
                return Ok(Vec::new());
            }
            let instant = to_nanos(now)?;
            self.connection
                .prepare_cached(
                    "UPDATE topics SET status = 'open', closed_at = NULL, updated_at = ?2 \
                 WHERE id = ?1",
                )?
                .execute(params![id.as_str(), instant])?;
            let reopened = self.thread_ids(
                "SELECT id FROM threads WHERE topic_id = ?1 AND closed_by_topic = 1 ORDER BY id",
                id.as_str(),
            )?;
            self.connection
                .prepare_cached(
                    "UPDATE threads SET status = 'open', closed_by_topic = 0, closed_at = NULL, \
                 updated_at = ?2 WHERE topic_id = ?1 AND closed_by_topic = 1",
                )?
                .execute(params![id.as_str(), instant])?;
            Ok(reopened)
        })
    }

    /// "Mark all read" on a topic: stamps it and acknowledges every open
    /// message in it, in any of its threads. Nothing is answered. Returns the
    /// messages this call acknowledged.
    pub fn mark_topic_read(
        &self,
        id: &TopicId,
        now: OffsetDateTime,
    ) -> Result<Vec<MessageId>, StoreError> {
        self.atomically(|| {
            self.topic(id)?;
            let instant = to_nanos(now)?;
            self.connection
                .prepare_cached(
                    "UPDATE topics SET marked_read_at = ?2, updated_at = ?2 WHERE id = ?1",
                )?
                .execute(params![id.as_str(), instant])?;
            self.acknowledge_open_messages("topic_id", id.as_str(), instant)
        })
    }

    // ---------------------------------------------------------------------
    // Threads
    // ---------------------------------------------------------------------

    /// Stores a new thread. A thread whose `default_for` is set becomes that
    /// conversation's default thread in its topic.
    ///
    /// # Errors
    ///
    /// - [`Conflict::ThreadId`] and [`Conflict::ThreadKey`] for duplicates.
    /// - [`Conflict::DefaultThread`] when the conversation already has a
    ///   default thread in the topic; a takeover goes through
    ///   [`Repository::insert_thread_with_takeover`].
    /// - [`StoreError::NotFound`] when the topic does not exist.
    pub fn insert_thread(&self, thread: &Thread) -> Result<(), StoreError> {
        self.atomically(|| {
            if self.exists(
                "SELECT 1 FROM threads WHERE id = ?1",
                params![thread.id.as_str()],
            )? {
                return Err(Conflict::ThreadId(thread.id.clone()).into());
            }
            if !self.exists(
                "SELECT 1 FROM topics WHERE id = ?1",
                params![thread.topic_id.as_str()],
            )? {
                return Err(StoreError::not_found(EntityKind::Topic, &thread.topic_id));
            }
            if let Some(key) = &thread.key
                && self.exists(
                    "SELECT 1 FROM threads WHERE topic_id = ?1 AND key = ?2",
                    params![thread.topic_id.as_str(), key],
                )?
            {
                return Err(Conflict::ThreadKey {
                    topic_id: thread.topic_id.clone(),
                    key: key.clone(),
                }
                .into());
            }
            if let Some(default_for) = &thread.default_for
                && self
                    .default_thread_link(
                        &default_for.agent,
                        &default_for.conversation_id,
                        &thread.topic_id,
                    )?
                    .is_some()
            {
                return Err(Conflict::DefaultThread {
                    agent: default_for.agent.clone(),
                    conversation_id: default_for.conversation_id.clone(),
                    topic_id: thread.topic_id.clone(),
                }
                .into());
            }
            self.connection.prepare_cached(
                "INSERT INTO threads (id, topic_id, key, title, summary, kind, implicit, status, \
                 closed_by_topic, muted, marked_read_at, created_by_agent, \
                 created_by_conversation_id, created_at, updated_at, closed_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            )?.execute(params![
                thread.id.as_str(),
                thread.topic_id.as_str(),
                thread.key,
                thread.title,
                thread.summary,
                thread.kind,
                thread.implicit,
                enum_text(&thread.status),
                thread.closed_by_topic,
                thread.muted,
                to_optional_nanos(thread.marked_read_at)?,
                thread.created_by.agent,
                thread.created_by.conversation_id,
                to_nanos(thread.created_at)?,
                to_nanos(thread.updated_at)?,
                to_optional_nanos(thread.closed_at)?,
            ])?;
            if let Some(default_for) = &thread.default_for {
                self.connection
                    .prepare_cached(
                        "INSERT INTO conversation_default_threads \
                     (agent, conversation_id, topic_id, thread_id, last_used) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    )?
                    .execute(params![
                        default_for.agent,
                        default_for.conversation_id,
                        thread.topic_id.as_str(),
                        thread.id.as_str(),
                        to_nanos(thread.created_at)?,
                    ])?;
            }
            Ok(())
        })
    }

    /// `POST /v1/threads`: stores `thread` unless its key exists in the topic,
    /// in which case the stored thread comes back unchanged.
    ///
    /// # Panics
    ///
    /// When `thread` has no key: only keyed threads upsert by key.
    pub fn upsert_thread(&self, thread: &Thread) -> Result<Upserted<Thread>, StoreError> {
        let Some(key) = thread.key.as_deref() else {
            panic!("a thread upserted by key needs a key");
        };
        self.atomically(|| match self.find_thread_by_key(&thread.topic_id, key)? {
            Some(existing) => Ok(Upserted::Existing(existing)),
            None => {
                self.insert_thread(thread)?;
                Ok(Upserted::Created(self.thread(&thread.id)?))
            }
        })
    }

    /// The takeover of ADR 0009: stores the explicit `thread` as the
    /// conversation's default thread in place of its implicit one, in one
    /// transaction. The implicit thread's messages move into the new thread,
    /// its mute carries over when `takeover.inherits_mute` says so, the
    /// conversation keeps its "last used" time, and the implicit thread is
    /// deleted. Returns how many messages moved.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] when the implicit thread is gone, and
    /// whatever [`Repository::insert_thread`] reports.
    ///
    /// # Panics
    ///
    /// When the thread to replace is not implicit, is in another topic than
    /// `thread`, or is not the default thread `thread.default_for` names: the
    /// hub derives the takeover from the stored state, so any of these is a
    /// bug.
    pub fn insert_thread_with_takeover(
        &self,
        thread: &Thread,
        takeover: &Takeover,
        now: OffsetDateTime,
    ) -> Result<u32, StoreError> {
        // The preconditions are checked before anything is opened or written.
        let replaced = self.thread(&takeover.replaces)?;
        assert!(
            replaced.implicit,
            "only an implicit thread can be taken over"
        );
        assert!(
            replaced.topic_id == thread.topic_id,
            "a takeover stays within one topic"
        );
        assert!(
            replaced.default_for.is_some() && replaced.default_for == thread.default_for,
            "the new thread must be the default of the conversation whose default it takes over"
        );
        self.atomically(|| {
            let last_used = self.last_used_of_default(&takeover.replaces)?;
            self.connection
                .prepare_cached("DELETE FROM conversation_default_threads WHERE thread_id = ?1")?
                .execute(params![takeover.replaces.as_str()])?;
            self.insert_thread(thread)?;
            self.connection
                .prepare_cached(
                    "UPDATE conversation_default_threads SET last_used = ?2 WHERE thread_id = ?1",
                )?
                .execute(params![thread.id.as_str(), to_nanos(last_used)?])?;
            if takeover.inherits_mute {
                self.connection
                    .prepare_cached("UPDATE threads SET muted = 1, updated_at = ?2 WHERE id = ?1")?
                    .execute(params![thread.id.as_str(), to_nanos(now)?])?;
            }
            let moved = self
                .connection
                .prepare_cached(
                    "UPDATE messages SET thread_id = ?2, updated_at = ?3 WHERE thread_id = ?1",
                )?
                .execute(params![
                    takeover.replaces.as_str(),
                    thread.id.as_str(),
                    to_nanos(now)?
                ])?;
            self.connection
                .prepare_cached("DELETE FROM threads WHERE id = ?1")?
                .execute(params![takeover.replaces.as_str()])?;
            u32::try_from(moved)
                .map_err(|_| StoreError::Constraint("too many messages moved".to_owned()))
        })
    }

    /// One thread, with the conversation it is the default of.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] when there is none.
    pub fn thread(&self, id: &ThreadId) -> Result<Thread, StoreError> {
        self.select_threads(" WHERE t.id = ?1", vec![Value::Text(id.to_string())])?
            .pop()
            .ok_or_else(|| StoreError::not_found(EntityKind::Thread, id))
    }

    /// The thread with this key in the topic, if any.
    pub fn find_thread_by_key(
        &self,
        topic_id: &TopicId,
        key: &str,
    ) -> Result<Option<Thread>, StoreError> {
        Ok(self
            .select_threads(
                " WHERE t.topic_id = ?1 AND t.key = ?2",
                vec![
                    Value::Text(topic_id.to_string()),
                    Value::Text(key.to_owned()),
                ],
            )?
            .pop())
    }

    /// Threads, oldest first, optionally of one topic and/or one status.
    pub fn list_threads(
        &self,
        topic_id: Option<&TopicId>,
        status: Option<GroupStatus>,
    ) -> Result<Vec<Thread>, StoreError> {
        let mut conditions = Vec::new();
        let mut values = Vec::new();
        if let Some(topic_id) = topic_id {
            values.push(Value::Text(topic_id.to_string()));
            conditions.push(format!("t.topic_id = ?{}", values.len()));
        }
        if let Some(status) = status {
            values.push(Value::Text(enum_text(&status)));
            conditions.push(format!("t.status = ?{}", values.len()));
        }
        let filter = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };
        self.select_threads(&format!("{filter} ORDER BY t.created_at, t.id"), values)
    }

    /// Saves what an agent or you may change on a thread: title, summary,
    /// kind, status, `closed_by_topic`, mute, mark-read time and the
    /// timestamps. Identity, key, topic (see [`Repository::move_thread`]),
    /// `implicit` and `created_by` never change.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] when the thread is not stored.
    pub fn update_thread(&self, thread: &Thread) -> Result<(), StoreError> {
        let changed = self
            .connection
            .prepare_cached(
                "UPDATE threads SET title = ?2, summary = ?3, kind = ?4, status = ?5, \
             closed_by_topic = ?6, muted = ?7, marked_read_at = ?8, updated_at = ?9, \
             closed_at = ?10 WHERE id = ?1",
            )?
            .execute(params![
                thread.id.as_str(),
                thread.title,
                thread.summary,
                thread.kind,
                enum_text(&thread.status),
                thread.closed_by_topic,
                thread.muted,
                to_optional_nanos(thread.marked_read_at)?,
                to_nanos(thread.updated_at)?,
                to_optional_nanos(thread.closed_at)?,
            ])?;
        if changed == 0 {
            return Err(StoreError::not_found(EntityKind::Thread, &thread.id));
        }
        Ok(())
    }

    /// A thread as the hub returns it, with its live counts.
    pub fn thread_record(&self, id: &ThreadId) -> Result<ThreadRecord, StoreError> {
        let thread = self.thread(id)?;
        self.record_of_thread(&thread)
    }

    /// Thread records, oldest first, optionally of one topic and/or status.
    pub fn list_thread_records(
        &self,
        topic_id: Option<&TopicId>,
        status: Option<GroupStatus>,
    ) -> Result<Vec<ThreadRecord>, StoreError> {
        self.list_threads(topic_id, status)?
            .iter()
            .map(|thread| self.record_of_thread(thread))
            .collect()
    }

    /// Closes a thread on its own (a later reopen of its topic leaves it
    /// closed). Closing a closed thread changes nothing. Returns whether this
    /// call closed it.
    pub fn close_thread(&self, id: &ThreadId, now: OffsetDateTime) -> Result<bool, StoreError> {
        self.atomically(|| {
            let thread = self.thread(id)?;
            if thread.status == GroupStatus::Closed {
                return Ok(false);
            }
            self.connection
                .prepare_cached(
                    "UPDATE threads SET status = 'closed', closed_by_topic = 0, closed_at = ?2, \
                 updated_at = ?2 WHERE id = ?1",
                )?
                .execute(params![id.as_str(), to_nanos(now)?])?;
            Ok(true)
        })
    }

    /// Reopens a thread and, when its topic is closed, the topic with it (a
    /// message to a closed thread does exactly this). Sibling threads that the
    /// topic's closing had closed stay closed, and from now on count as
    /// closed on their own.
    pub fn reopen_thread(
        &self,
        id: &ThreadId,
        now: OffsetDateTime,
    ) -> Result<ThreadReopened, StoreError> {
        self.atomically(|| {
            let thread = self.thread(id)?;
            let instant = to_nanos(now)?;
            let mut reopened = ThreadReopened {
                thread: false,
                topic: false,
            };
            if thread.status == GroupStatus::Closed {
                self.connection.prepare_cached(
                    "UPDATE threads SET status = 'open', closed_by_topic = 0, closed_at = NULL, \
                     updated_at = ?2 WHERE id = ?1",
                )?.execute(params![id.as_str(), instant])?;
                reopened.thread = true;
            }
            let topic = self.topic(&thread.topic_id)?;
            if topic.status == GroupStatus::Closed {
                self.connection
                    .prepare_cached(
                        "UPDATE topics SET status = 'open', closed_at = NULL, updated_at = ?2 \
                     WHERE id = ?1",
                    )?
                    .execute(params![topic.id.as_str(), instant])?;
                self.connection
                    .prepare_cached("UPDATE threads SET closed_by_topic = 0 WHERE topic_id = ?1")?
                    .execute(params![topic.id.as_str()])?;
                reopened.topic = true;
            }
            Ok(reopened)
        })
    }

    /// "Mark all read" on a thread: stamps it and acknowledges every open
    /// message in it. Returns the messages this call acknowledged.
    pub fn mark_thread_read(
        &self,
        id: &ThreadId,
        now: OffsetDateTime,
    ) -> Result<Vec<MessageId>, StoreError> {
        self.atomically(|| {
            self.thread(id)?;
            let instant = to_nanos(now)?;
            self.connection
                .prepare_cached(
                    "UPDATE threads SET marked_read_at = ?2, updated_at = ?2 WHERE id = ?1",
                )?
                .execute(params![id.as_str(), instant])?;
            self.acknowledge_open_messages("thread_id", id.as_str(), instant)
        })
    }

    /// Moves a thread, with its messages, to another topic: every message's
    /// `topic_id` follows, and a default-thread link follows too. Moving a
    /// thread to its own topic changes nothing.
    ///
    /// # Errors
    ///
    /// - [`StoreError::NotFound`] for an unknown thread or topic.
    /// - [`Conflict::ThreadKey`] when the target topic has a thread with the
    ///   same key.
    /// - [`Conflict::DefaultThread`] when the thread is a conversation's
    ///   default and that conversation already has one in the target topic.
    pub fn move_thread(
        &self,
        id: &ThreadId,
        target: &TopicId,
        now: OffsetDateTime,
    ) -> Result<(), StoreError> {
        self.atomically(|| {
            let thread = self.thread(id)?;
            self.topic(target)?;
            if thread.topic_id == *target {
                return Ok(());
            }
            if let Some(key) = &thread.key
                && self.find_thread_by_key(target, key)?.is_some()
            {
                return Err(Conflict::ThreadKey {
                    topic_id: target.clone(),
                    key: key.clone(),
                }
                .into());
            }
            if let Some(default_for) = &thread.default_for
                && self
                    .default_thread_link(&default_for.agent, &default_for.conversation_id, target)?
                    .is_some()
            {
                return Err(Conflict::DefaultThread {
                    agent: default_for.agent.clone(),
                    conversation_id: default_for.conversation_id.clone(),
                    topic_id: target.clone(),
                }
                .into());
            }
            // The composite foreign keys of the messages and of the default-thread
            // link cascade the new topic id. A thread that moves is no longer held
            // closed by the topic it left.
            self.connection
                .prepare_cached(
                    "UPDATE threads SET topic_id = ?2, closed_by_topic = 0, updated_at = ?3 \
                     WHERE id = ?1",
                )?
                .execute(params![id.as_str(), target.as_str(), to_nanos(now)?])?;
            Ok(())
        })
    }

    /// Moves one message to another thread (and so to that thread's topic).
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] for an unknown message or thread.
    pub fn move_message_to_thread(
        &self,
        id: &MessageId,
        thread_id: &ThreadId,
        now: OffsetDateTime,
    ) -> Result<(), StoreError> {
        self.atomically(|| {
            self.message_exists(id)?;
            let topic_id = self.thread_topic_id(thread_id)?;
            self.connection
                .prepare_cached(
                    "UPDATE messages SET thread_id = ?2, topic_id = ?3, updated_at = ?4 \
                 WHERE id = ?1",
                )?
                .execute(params![
                    id.as_str(),
                    thread_id.as_str(),
                    topic_id.as_str(),
                    to_nanos(now)?
                ])?;
            Ok(())
        })
    }

    // ---------------------------------------------------------------------
    // Default threads and the snapshot
    // ---------------------------------------------------------------------

    /// The conversation's default thread in a topic, if it has one.
    pub fn default_thread_link(
        &self,
        agent: &str,
        conversation_id: &str,
        topic_id: &TopicId,
    ) -> Result<Option<DefaultThreadLink>, StoreError> {
        Ok(self
            .select_links(
                " WHERE agent = ?1 AND conversation_id = ?2 AND topic_id = ?3",
                vec![
                    Value::Text(agent.to_owned()),
                    Value::Text(conversation_id.to_owned()),
                    Value::Text(topic_id.to_string()),
                ],
            )?
            .pop())
    }

    /// Every conversation's default threads.
    pub fn default_links(&self) -> Result<Vec<DefaultThreadLink>, StoreError> {
        self.select_links(" ORDER BY last_used, thread_id", Vec::new())
    }

    /// Records that a default message landed in the conversation's default
    /// thread at `at`; the latest such time decides where the conversation's
    /// topicless messages go.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] when the conversation has no default thread in
    /// the topic.
    pub fn touch_default_thread(
        &self,
        agent: &str,
        conversation_id: &str,
        topic_id: &TopicId,
        at: OffsetDateTime,
    ) -> Result<(), StoreError> {
        let changed = self
            .connection
            .prepare_cached(
                "UPDATE conversation_default_threads SET last_used = ?4 \
             WHERE agent = ?1 AND conversation_id = ?2 AND topic_id = ?3",
            )?
            .execute(params![
                agent,
                conversation_id,
                topic_id.as_str(),
                to_nanos(at)?
            ])?;
        if changed == 0 {
            return Err(StoreError::not_found(
                EntityKind::Thread,
                format!("default thread of {agent}/{conversation_id} in {topic_id}"),
            ));
        }
        Ok(())
    }

    /// Everything `gentbox_core::resolve` needs to resolve a message: all
    /// topics, all threads and all default links.
    pub fn grouping_snapshot(&self) -> Result<GroupingSnapshot, StoreError> {
        Ok(GroupingSnapshot {
            topics: self.list_topics(None)?,
            threads: self.list_threads(None, None)?,
            default_links: self.default_links()?,
        })
    }

    // ---------------------------------------------------------------------
    // Internals
    // ---------------------------------------------------------------------

    pub(crate) fn thread_topic_id(&self, id: &ThreadId) -> Result<TopicId, StoreError> {
        self.connection
            .prepare_cached("SELECT topic_id FROM threads WHERE id = ?1")?
            .query_row(params![id.as_str()], |row| row.get::<_, String>(0))
            .optional()?
            .map(TopicId::new)
            .ok_or_else(|| StoreError::not_found(EntityKind::Thread, id))
    }

    fn last_used_of_default(&self, thread_id: &ThreadId) -> Result<OffsetDateTime, StoreError> {
        let nanos: i64 = self
            .connection
            .prepare_cached(
                "SELECT last_used FROM conversation_default_threads WHERE thread_id = ?1",
            )?
            .query_row(params![thread_id.as_str()], |row| row.get(0))?;
        from_nanos("last_used", nanos)
    }

    fn acknowledge_open_messages(
        &self,
        scope_column: &'static str,
        scope_id: &str,
        instant: i64,
    ) -> Result<Vec<MessageId>, StoreError> {
        // `scope_column` is one of two literals chosen by the callers above.
        let sql = format!(
            "UPDATE messages SET state = 'acknowledged', updated_at = ?2 \
             WHERE {scope_column} = ?1 AND state = 'open' RETURNING id"
        );
        let mut statement = self.connection.prepare_cached(&sql)?;
        let mut rows = statement.query(params![scope_id, instant])?;
        let mut acknowledged = Vec::new();
        while let Some(row) = rows.next()? {
            acknowledged.push(MessageId::new(row.get::<_, String>(0)?));
        }
        acknowledged.sort();
        Ok(acknowledged)
    }

    fn thread_ids(&self, sql: &str, parameter: &str) -> Result<Vec<ThreadId>, StoreError> {
        let mut statement = self.connection.prepare_cached(sql)?;
        let mut rows = statement.query(params![parameter])?;
        let mut ids = Vec::new();
        while let Some(row) = rows.next()? {
            ids.push(ThreadId::new(row.get::<_, String>(0)?));
        }
        Ok(ids)
    }

    fn count(&self, sql: &str, parameter: &str) -> Result<u32, StoreError> {
        let count: i64 = self
            .connection
            .prepare_cached(sql)?
            .query_row(params![parameter], |row| row.get(0))?;
        u32::try_from(count).map_err(|_| StoreError::corrupt("count", count))
    }

    fn record_of_topic(&self, topic: &Topic) -> Result<TopicRecord, StoreError> {
        let open_messages = self.count(
            "SELECT COUNT(*) FROM messages WHERE topic_id = ?1 \
             AND state IN ('open', 'acknowledged')",
            topic.id.as_str(),
        )?;
        let open_threads = self.count(
            "SELECT COUNT(*) FROM threads WHERE topic_id = ?1 AND status = 'open'",
            topic.id.as_str(),
        )?;
        Ok(TopicRecord::from_topic(topic, open_messages, open_threads))
    }

    fn record_of_thread(&self, thread: &Thread) -> Result<ThreadRecord, StoreError> {
        let open_messages = self.count(
            "SELECT COUNT(*) FROM messages WHERE thread_id = ?1 \
             AND state IN ('open', 'acknowledged')",
            thread.id.as_str(),
        )?;
        let conversations = self.count(
            "SELECT COUNT(*) FROM (SELECT DISTINCT source_agent, conversation_id \
             FROM messages WHERE thread_id = ?1)",
            thread.id.as_str(),
        )?;
        Ok(ThreadRecord::from_thread(
            thread,
            open_messages,
            conversations,
        ))
    }

    fn select_topics(&self, tail: &str, values: Vec<Value>) -> Result<Vec<Topic>, StoreError> {
        let sql = format!("{SELECT_TOPIC}{tail}");
        let mut statement = self.connection.prepare_cached(&sql)?;
        let mut rows = statement.query(params_from_iter(values))?;
        let mut topics = Vec::new();
        while let Some(row) = rows.next()? {
            topics.push(decode_topic(row)?);
        }
        Ok(topics)
    }

    fn select_threads(&self, tail: &str, values: Vec<Value>) -> Result<Vec<Thread>, StoreError> {
        let sql = format!("{SELECT_THREAD}{tail}");
        let mut statement = self.connection.prepare_cached(&sql)?;
        let mut rows = statement.query(params_from_iter(values))?;
        let mut threads = Vec::new();
        while let Some(row) = rows.next()? {
            threads.push(decode_thread(row)?);
        }
        Ok(threads)
    }

    fn select_links(
        &self,
        tail: &str,
        values: Vec<Value>,
    ) -> Result<Vec<DefaultThreadLink>, StoreError> {
        let sql = format!(
            "SELECT agent, conversation_id, topic_id, thread_id, last_used \
             FROM conversation_default_threads{tail}"
        );
        let mut statement = self.connection.prepare_cached(&sql)?;
        let mut rows = statement.query(params_from_iter(values))?;
        let mut links = Vec::new();
        while let Some(row) = rows.next()? {
            links.push(DefaultThreadLink {
                agent: row.get(0)?,
                conversation_id: row.get(1)?,
                topic_id: TopicId::new(row.get::<_, String>(2)?),
                thread_id: ThreadId::new(row.get::<_, String>(3)?),
                last_used: from_nanos("last_used", row.get(4)?)?,
            });
        }
        Ok(links)
    }
}

fn decode_topic(row: &Row<'_>) -> Result<Topic, StoreError> {
    Ok(Topic {
        id: TopicId::new(row.get::<_, String>("id")?),
        key: row.get("key")?,
        title: row.get("title")?,
        summary: row.get("summary")?,
        kind: row.get("kind")?,
        style: from_optional_json::<TopicStyle>(
            "style",
            row.get::<_, Option<String>>("style")?.as_deref(),
        )?,
        implicit: row.get("implicit")?,
        status: parse_enum("status", &row.get::<_, String>("status")?)?,
        muted: row.get("muted")?,
        marked_read_at: from_optional_nanos("marked_read_at", row.get("marked_read_at")?)?,
        created_by: CreatedBy {
            agent: row.get("created_by_agent")?,
            conversation_id: row.get("created_by_conversation_id")?,
        },
        created_at: from_nanos("created_at", row.get("created_at")?)?,
        updated_at: from_nanos("updated_at", row.get("updated_at")?)?,
        closed_at: from_optional_nanos("closed_at", row.get("closed_at")?)?,
    })
}

fn decode_thread(row: &Row<'_>) -> Result<Thread, StoreError> {
    let default_for = match row.get::<_, Option<String>>("default_agent")? {
        None => None,
        Some(agent) => Some(DefaultFor {
            agent,
            conversation_id: row.get("default_conversation_id")?,
        }),
    };
    Ok(Thread {
        id: ThreadId::new(row.get::<_, String>("id")?),
        topic_id: TopicId::new(row.get::<_, String>("topic_id")?),
        key: row.get("key")?,
        title: row.get("title")?,
        summary: row.get("summary")?,
        kind: row.get("kind")?,
        implicit: row.get("implicit")?,
        status: parse_enum("status", &row.get::<_, String>("status")?)?,
        closed_by_topic: row.get("closed_by_topic")?,
        muted: row.get("muted")?,
        marked_read_at: from_optional_nanos("marked_read_at", row.get("marked_read_at")?)?,
        default_for,
        created_by: CreatedBy {
            agent: row.get("created_by_agent")?,
            conversation_id: row.get("created_by_conversation_id")?,
        },
        created_at: from_nanos("created_at", row.get("created_at")?)?,
        updated_at: from_nanos("updated_at", row.get("updated_at")?)?,
        closed_at: from_optional_nanos("closed_at", row.get("closed_at")?)?,
    })
}

const SELECT_TOPIC: &str = "\
SELECT id, key, title, summary, kind, style, implicit, status, muted, marked_read_at, \
created_by_agent, created_by_conversation_id, created_at, updated_at, closed_at FROM topics";

const SELECT_THREAD: &str = "\
SELECT t.id, t.topic_id, t.key, t.title, t.summary, t.kind, t.implicit, t.status, \
t.closed_by_topic, t.muted, t.marked_read_at, t.created_by_agent, t.created_by_conversation_id, \
t.created_at, t.updated_at, t.closed_at, \
d.agent AS default_agent, d.conversation_id AS default_conversation_id \
FROM threads t LEFT JOIN conversation_default_threads d ON d.thread_id = t.id";
