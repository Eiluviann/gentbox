//! The repository: messages, conversations, actions, responses and the
//! delivery log. Topics and threads live in [`crate::groups`], retention in
//! [`crate::retention`]; all three are methods of [`Repository`].
//!
//! The API takes and returns `gentbox-core` types. Not-found and constraint
//! outcomes come back as [`StoreError`] values; a caller handing over data
//! that breaks a documented precondition (a message whose topic is not its
//! thread's topic) gets a panic, because only a bug can cause it.

use gentbox_core::classify::FieldSources;
use gentbox_core::ids::{ConversationKey, MessageId, ThreadId, TopicId};
use gentbox_core::lifecycle::{Lifecycle, LifecycleState, RecordedAnswer, ResolutionReason};
use gentbox_core::message::{
    Action, ActionKind, ActionStep, Blocking, Conversation, FocusTarget, Message, MessageScope,
    MessageType, Metadata, Source,
};
use gentbox_core::rescind::RescindReason;
use gentbox_core::response::AnsweredVia;
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Row, named_params, params, params_from_iter};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::codec::{
    enum_text, from_json, from_nanos, from_optional_json, from_optional_nanos, parse_enum,
    parse_optional_enum, to_json, to_nanos, to_optional_json, to_optional_nanos,
};
use crate::error::{Conflict, EntityKind, StoreError};

/// Reads and writes against a connection or an open transaction.
///
/// Obtained from [`crate::Store::repository`] (each call commits by itself) or
/// handed to the closure of [`crate::Store::transaction`] (all calls commit
/// together). Every method that touches more than one row is atomic on its
/// own, inside or outside a transaction.
#[derive(Debug, Clone, Copy)]
pub struct Repository<'connection> {
    pub(crate) connection: &'connection Connection,
}

/// What a delivery-log entry records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryEvent {
    /// The hub put the message on a surface.
    Delivered,
    /// A still-open message pinged again.
    Repinged,
    /// Classification changed a field to make the three consistent.
    Normalized,
    /// The hub warned the sender about something it changed.
    Warned,
    /// The message was withdrawn from the surfaces.
    Withdrawn,
}

/// One entry of the delivery log: what the hub did with a message and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryRecord {
    /// The message.
    pub message_id: MessageId,
    /// When it happened.
    pub recorded_at: OffsetDateTime,
    /// What happened.
    pub event: DeliveryEvent,
    /// The surface involved (`popup`, `toast`, `banner`, `inbox`, …).
    pub surface: Option<String>,
    /// Plain-words detail: the normalization, the warning, the reason.
    pub detail: Option<String>,
}

/// Which messages [`Repository::list_messages`] returns. The default filter
/// lists every live (open or acknowledged) message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MessageFilter {
    /// States to include; `None` means live: open and acknowledged.
    pub states: Option<Vec<LifecycleState>>,
    /// Only this conversation.
    pub conversation: Option<ConversationKey>,
    /// Only this type.
    pub message_type: Option<MessageType>,
    /// Only urgent or only non-urgent messages.
    pub urgent: Option<bool>,
    /// Only this blocking level.
    pub blocking: Option<Blocking>,
    /// Only this topic.
    pub topic_id: Option<TopicId>,
    /// Only this thread.
    pub thread_id: Option<ThreadId>,
    /// Leave out messages snoozed past this instant: the inbox's open list.
    pub hide_snoozed_at: Option<OffsetDateTime>,
    /// At most this many, newest first.
    pub limit: Option<u32>,
}

impl<'connection> Repository<'connection> {
    pub(crate) fn new(connection: &'connection Connection) -> Self {
        Self { connection }
    }

    /// Runs `work` inside a savepoint: everything it wrote is kept when it
    /// returns `Ok` and undone when it returns `Err`.
    pub(crate) fn atomically<T>(
        &self,
        work: impl FnOnce() -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        self.connection.execute_batch("SAVEPOINT gentbox_atomic")?;
        match work() {
            Ok(value) => {
                self.connection.execute_batch("RELEASE gentbox_atomic")?;
                Ok(value)
            }
            Err(error) => {
                self.connection
                    .execute_batch("ROLLBACK TO gentbox_atomic; RELEASE gentbox_atomic")?;
                Err(error)
            }
        }
    }

    pub(crate) fn exists(
        &self,
        sql: &str,
        parameters: impl rusqlite::Params,
    ) -> Result<bool, StoreError> {
        Ok(self.connection.prepare_cached(sql)?.exists(parameters)?)
    }

    // ---------------------------------------------------------------------
    // Messages
    // ---------------------------------------------------------------------

    /// Stores a new message with its actions and, when it already carries one,
    /// its recorded answer. The conversation row is created or refreshed from
    /// `message.conversation`.
    ///
    /// `now` stamps `closed_at` when the message is stored already closed.
    ///
    /// # Errors
    ///
    /// - [`Conflict::MessageId`] and [`Conflict::IdempotencyKey`] for a
    ///   duplicate.
    /// - [`StoreError::NotFound`] when the thread does not exist.
    ///
    /// # Panics
    ///
    /// When `message.topic_id` is not the topic of `message.thread_id`: the
    /// hub resolves both together, so a mismatch is a bug.
    pub fn insert_message(&self, message: &Message, now: OffsetDateTime) -> Result<(), StoreError> {
        self.atomically(|| {
            self.check_message_is_new(message)?;
            self.check_message_grouping(message)?;
            self.upsert_conversation(&message.conversation_key(), &message.conversation, now)?;
            self.write_message_row(message, now)?;
            self.write_actions(message)?;
            self.write_response(&message.id, message.lifecycle.answer.as_ref())
        })
    }

    /// Saves the mutable parts of a stored message: classification, content,
    /// plan, buttons, lifecycle, recorded answer and repeat count. Its
    /// identity, sender, conversation, grouping and receive time never change
    /// here (see [`Repository::move_message_to_thread`]).
    ///
    /// `now` stamps `closed_at` the first time the message is closed.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] when the message is not stored.
    pub fn update_message(&self, message: &Message, now: OffsetDateTime) -> Result<(), StoreError> {
        self.atomically(|| {
            let changed =
                self.connection
                    .prepare_cached(UPDATE_MESSAGE)?
                    .execute(named_params! {
                        ":id": message.id.as_str(),
                        ":type": enum_text(&message.message_type),
                        ":urgent": message.urgent,
                        ":blocking": enum_text(&message.blocking),
                        ":field_sources": to_json("field_sources", &message.field_sources)?,
                        ":muted_by_rule": message.muted_by_rule,
                        ":event": message.event,
                        ":title": message.title,
                        ":summary": message.summary,
                        ":body": message.body,
                        ":steps": to_json("steps", &message.steps)?,
                        ":steps_completed": to_json("steps_completed", &message.steps_completed)?,
                        ":scope": message.scope.as_ref().map(enum_text),
                        ":awaits_response": message.awaits_response,
                        ":correlation_key": message.correlation_key,
                        ":sensitive": message.sensitive,
                        ":created_at": to_optional_nanos(message.created_at)?,
                        ":expires_at": to_nanos(message.expires_at)?,
                        ":metadata": to_json("metadata", &message.metadata)?,
                        ":repeat_count": message.repeat_count,
                        ":state": enum_text(&message.lifecycle.state),
                        ":snoozed_until": to_optional_nanos(message.lifecycle.snoozed_until)?,
                        ":deferred_at": to_optional_nanos(message.lifecycle.deferred_at)?,
                        ":resolution": resolution_kind(message.lifecycle.resolution),
                        ":rescind_reason": rescind_reason(message.lifecycle.resolution),
                        ":is_closed": message.lifecycle.state.is_closed(),
                        ":now": to_nanos(now)?,
                    })?;
            if changed == 0 {
                return Err(StoreError::not_found(EntityKind::Message, &message.id));
            }
            self.write_actions(message)?;
            self.write_response(&message.id, message.lifecycle.answer.as_ref())
        })
    }

    /// One stored message.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] when there is none.
    pub fn message(&self, id: &MessageId) -> Result<Message, StoreError> {
        self.find_message(id)?
            .ok_or_else(|| StoreError::not_found(EntityKind::Message, id))
    }

    /// One stored message, or `None`.
    pub fn find_message(&self, id: &MessageId) -> Result<Option<Message>, StoreError> {
        self.select_messages(" WHERE m.id = ?1", vec![Value::Text(id.to_string())])
            .map(|mut messages| messages.pop())
    }

    /// The message a retried send created, for the idempotent 200.
    pub fn find_message_by_idempotency_key(
        &self,
        agent: &str,
        idempotency_key: &str,
    ) -> Result<Option<Message>, StoreError> {
        self.select_messages(
            " WHERE m.source_agent = ?1 AND m.idempotency_key = ?2",
            vec![
                Value::Text(agent.to_owned()),
                Value::Text(idempotency_key.to_owned()),
            ],
        )
        .map(|mut messages| messages.pop())
    }

    /// Messages matching `filter`, newest received first.
    pub fn list_messages(&self, filter: &MessageFilter) -> Result<Vec<Message>, StoreError> {
        let mut query = WhereClause::default();
        let states = filter
            .states
            .clone()
            .unwrap_or_else(|| vec![LifecycleState::Open, LifecycleState::Acknowledged]);
        let state_names = states.iter().map(enum_text).map(Value::Text).collect();
        query.push_any_of("m.state", state_names);
        if let Some(conversation) = &filter.conversation {
            query.push_equal("m.source_agent", Value::Text(conversation.agent.clone()));
            query.push_equal(
                "m.conversation_id",
                Value::Text(conversation.conversation_id.clone()),
            );
        }
        if let Some(message_type) = filter.message_type {
            query.push_equal("m.type", Value::Text(enum_text(&message_type)));
        }
        if let Some(urgent) = filter.urgent {
            query.push_equal("m.urgent", Value::Integer(i64::from(urgent)));
        }
        if let Some(blocking) = filter.blocking {
            query.push_equal("m.blocking", Value::Text(enum_text(&blocking)));
        }
        if let Some(topic_id) = &filter.topic_id {
            query.push_equal("m.topic_id", Value::Text(topic_id.to_string()));
        }
        if let Some(thread_id) = &filter.thread_id {
            query.push_equal("m.thread_id", Value::Text(thread_id.to_string()));
        }
        if let Some(instant) = filter.hide_snoozed_at {
            query.push_snoozed_before(to_nanos(instant)?);
        }
        let mut tail = query.render();
        tail.push_str(" ORDER BY m.received_at DESC, m.id DESC");
        if let Some(limit) = filter.limit {
            tail.push_str(&format!(" LIMIT {limit}"));
        }
        self.select_messages(&tail, query.values)
    }

    /// Live messages whose `expires_at` has passed at `now`, oldest first: the
    /// hub expires them through the lifecycle.
    pub fn list_live_messages_expired_by(
        &self,
        now: OffsetDateTime,
    ) -> Result<Vec<MessageId>, StoreError> {
        self.select_ids(
            "SELECT id FROM messages WHERE state IN ('open', 'acknowledged') \
             AND expires_at <= ?1 ORDER BY expires_at, id",
            to_nanos(now)?,
        )
    }

    /// Acknowledged messages whose snooze has ended at `now`, oldest first:
    /// the hub returns them to `open` through the lifecycle.
    pub fn list_snoozes_ended_by(&self, now: OffsetDateTime) -> Result<Vec<MessageId>, StoreError> {
        self.select_ids(
            "SELECT id FROM messages WHERE state = 'acknowledged' \
             AND snoozed_until IS NOT NULL AND snoozed_until <= ?1 \
             ORDER BY snoozed_until, id",
            to_nanos(now)?,
        )
    }

    /// Records that `successor` replaced `replaced`: the supersede chain. The
    /// replaced message is rescinded separately, through the lifecycle, with
    /// [`RescindReason::Superseded`].
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] when either message is not stored.
    ///
    /// # Panics
    ///
    /// When a message would succeed itself.
    pub fn record_supersession(
        &self,
        replaced: &MessageId,
        successor: &MessageId,
    ) -> Result<(), StoreError> {
        assert!(replaced != successor, "a message cannot supersede itself");
        self.atomically(|| {
            self.message_exists(successor)?;
            let changed = self
                .connection
                .prepare_cached("UPDATE messages SET superseded_by = ?2 WHERE id = ?1")?
                .execute(params![replaced.as_str(), successor.as_str()])?;
            if changed == 0 {
                return Err(StoreError::not_found(EntityKind::Message, replaced));
            }
            Ok(())
        })
    }

    /// The message that directly replaced `id`, if any.
    pub fn superseded_by(&self, id: &MessageId) -> Result<Option<MessageId>, StoreError> {
        let successor: Option<Option<String>> = self
            .connection
            .prepare_cached("SELECT superseded_by FROM messages WHERE id = ?1")?
            .query_row(params![id.as_str()], |row| row.get(0))
            .optional()?;
        match successor {
            None => Err(StoreError::not_found(EntityKind::Message, id)),
            Some(successor) => Ok(successor.map(MessageId::new)),
        }
    }

    /// Every message that followed `id` in its supersede chain, nearest
    /// first. Empty when nothing replaced it.
    pub fn supersede_chain(&self, id: &MessageId) -> Result<Vec<MessageId>, StoreError> {
        let mut chain = Vec::new();
        let mut current = id.clone();
        while let Some(next) = self.superseded_by(&current)? {
            // The chain only ever points at newer rows, so it cannot loop;
            // the guard keeps a corrupt file from hanging the hub.
            if next == *id || chain.contains(&next) {
                return Err(StoreError::corrupt("superseded_by", "supersede cycle"));
            }
            chain.push(next.clone());
            current = next;
        }
        Ok(chain)
    }

    /// When the message reached a terminal state, as retention counts it.
    pub fn closed_at(&self, id: &MessageId) -> Result<Option<OffsetDateTime>, StoreError> {
        let closed: Option<Option<i64>> = self
            .connection
            .prepare_cached("SELECT closed_at FROM messages WHERE id = ?1")?
            .query_row(params![id.as_str()], |row| row.get(0))
            .optional()?;
        match closed {
            None => Err(StoreError::not_found(EntityKind::Message, id)),
            Some(nanos) => from_optional_nanos("closed_at", nanos),
        }
    }

    // ---------------------------------------------------------------------
    // Conversations
    // ---------------------------------------------------------------------

    /// Registers a conversation or refreshes it (`SessionStart`, renames).
    /// Fields the new value leaves out keep their stored value.
    pub fn upsert_conversation(
        &self,
        key: &ConversationKey,
        conversation: &Conversation,
        now: OffsetDateTime,
    ) -> Result<(), StoreError> {
        assert!(
            conversation.id == key.conversation_id,
            "the conversation key and the conversation disagree on the id"
        );
        self.connection
            .prepare_cached(UPSERT_CONVERSATION)?
            .execute(named_params! {
                ":agent": key.agent,
                ":conversation_id": key.conversation_id,
                ":title": conversation.title,
                ":cwd": conversation.cwd,
                ":focus": to_optional_json("focus", conversation.focus.as_ref())?,
                ":updated_at": to_nanos(now)?,
            })?;
        Ok(())
    }

    /// A stored conversation.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] when the conversation was never registered.
    pub fn conversation(&self, key: &ConversationKey) -> Result<Conversation, StoreError> {
        let row: Option<(Option<String>, Option<String>, Option<String>)> = self
            .connection
            .prepare_cached(
                "SELECT title, cwd, focus FROM conversations \
                 WHERE agent = ?1 AND conversation_id = ?2",
            )?
            .query_row(params![key.agent, key.conversation_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .optional()?;
        let (title, cwd, focus) = row.ok_or_else(|| {
            StoreError::not_found(
                EntityKind::Conversation,
                format!("{}/{}", key.agent, key.conversation_id),
            )
        })?;
        Ok(Conversation {
            id: key.conversation_id.clone(),
            title,
            cwd,
            focus: from_optional_json::<FocusTarget>("focus", focus.as_deref())?,
        })
    }

    // ---------------------------------------------------------------------
    // Delivery log
    // ---------------------------------------------------------------------

    /// Appends an entry to the delivery log.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] when the message is not stored.
    pub fn append_delivery(&self, record: &DeliveryRecord) -> Result<(), StoreError> {
        self.message_exists(&record.message_id)?;
        self.connection
            .prepare_cached(
                "INSERT INTO deliveries (message_id, recorded_at, event, surface, detail) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            )?
            .execute(params![
                record.message_id.as_str(),
                to_nanos(record.recorded_at)?,
                enum_text(&record.event),
                record.surface,
                record.detail,
            ])?;
        Ok(())
    }

    /// A message's delivery log, oldest entry first.
    pub fn deliveries(&self, message_id: &MessageId) -> Result<Vec<DeliveryRecord>, StoreError> {
        self.message_exists(message_id)?;
        let mut statement = self.connection.prepare_cached(
            "SELECT recorded_at, event, surface, detail FROM deliveries \
             WHERE message_id = ?1 ORDER BY sequence",
        )?;
        let mut rows = statement.query(params![message_id.as_str()])?;
        let mut records = Vec::new();
        while let Some(row) = rows.next()? {
            records.push(DeliveryRecord {
                message_id: message_id.clone(),
                recorded_at: from_nanos("recorded_at", row.get(0)?)?,
                event: parse_enum("event", &row.get::<_, String>(1)?)?,
                surface: row.get(2)?,
                detail: row.get(3)?,
            });
        }
        Ok(records)
    }

    // ---------------------------------------------------------------------
    // Internals
    // ---------------------------------------------------------------------

    pub(crate) fn message_exists(&self, id: &MessageId) -> Result<(), StoreError> {
        if self.exists("SELECT 1 FROM messages WHERE id = ?1", params![id.as_str()])? {
            Ok(())
        } else {
            Err(StoreError::not_found(EntityKind::Message, id))
        }
    }

    fn check_message_is_new(&self, message: &Message) -> Result<(), StoreError> {
        if self.exists(
            "SELECT 1 FROM messages WHERE id = ?1",
            params![message.id.as_str()],
        )? {
            return Err(Conflict::MessageId(message.id.clone()).into());
        }
        if let Some(key) = &message.idempotency_key
            && self.exists(
                "SELECT 1 FROM messages WHERE source_agent = ?1 AND idempotency_key = ?2",
                params![message.source.agent, key],
            )?
        {
            return Err(Conflict::IdempotencyKey {
                agent: message.source.agent.clone(),
                key: key.clone(),
            }
            .into());
        }
        Ok(())
    }

    fn check_message_grouping(&self, message: &Message) -> Result<(), StoreError> {
        let topic_id = self.thread_topic_id(&message.thread_id)?;
        assert!(
            topic_id == message.topic_id,
            "message {} names topic {} but its thread {} is in topic {}",
            message.id,
            message.topic_id,
            message.thread_id,
            topic_id
        );
        Ok(())
    }

    fn write_message_row(&self, message: &Message, now: OffsetDateTime) -> Result<(), StoreError> {
        let lifecycle = &message.lifecycle;
        self.connection
            .prepare_cached(INSERT_MESSAGE)?
            .execute(named_params! {
                ":id": message.id.as_str(),
                ":idempotency_key": message.idempotency_key,
                ":source_agent": message.source.agent,
                ":source_display_name": message.source.display_name,
                ":source_version": message.source.version,
                ":source_host": message.source.host,
                ":conversation_id": message.conversation.id,
                ":topic_id": message.topic_id.as_str(),
                ":thread_id": message.thread_id.as_str(),
                ":type": enum_text(&message.message_type),
                ":urgent": message.urgent,
                ":blocking": enum_text(&message.blocking),
                ":field_sources": to_json("field_sources", &message.field_sources)?,
                ":muted_by_rule": message.muted_by_rule,
                ":event": message.event,
                ":title": message.title,
                ":summary": message.summary,
                ":body": message.body,
                ":steps": to_json("steps", &message.steps)?,
                ":steps_completed": to_json("steps_completed", &message.steps_completed)?,
                ":scope": message.scope.as_ref().map(enum_text),
                ":awaits_response": message.awaits_response,
                ":correlation_key": message.correlation_key,
                ":sensitive": message.sensitive,
                ":created_at": to_optional_nanos(message.created_at)?,
                ":received_at": to_nanos(message.received_at)?,
                ":expires_at": to_nanos(message.expires_at)?,
                ":metadata": to_json("metadata", &message.metadata)?,
                ":repeat_count": message.repeat_count,
                ":state": enum_text(&lifecycle.state),
                ":snoozed_until": to_optional_nanos(lifecycle.snoozed_until)?,
                ":deferred_at": to_optional_nanos(lifecycle.deferred_at)?,
                ":resolution": resolution_kind(lifecycle.resolution),
                ":rescind_reason": rescind_reason(lifecycle.resolution),
                ":closed_at": lifecycle.state.is_closed().then_some(to_nanos(now)?),
                ":updated_at": to_nanos(now)?,
            })?;
        Ok(())
    }

    fn write_actions(&self, message: &Message) -> Result<(), StoreError> {
        self.connection
            .prepare_cached("DELETE FROM actions WHERE message_id = ?1")?
            .execute(params![message.id.as_str()])?;
        let mut insert = self.connection.prepare_cached(
            "INSERT INTO actions \
             (message_id, position, action_id, label, kind, value, placeholder, destructive) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        for (position, action) in message.actions.iter().enumerate() {
            let position = i64::try_from(position)
                .map_err(|_| StoreError::Constraint("too many actions".to_owned()))?;
            insert.execute(params![
                message.id.as_str(),
                position,
                action.id,
                action.label,
                enum_text(&action.kind),
                action.value,
                action.placeholder,
                action.destructive,
            ])?;
        }
        Ok(())
    }

    fn write_response(
        &self,
        id: &MessageId,
        answer: Option<&RecordedAnswer>,
    ) -> Result<(), StoreError> {
        self.connection
            .prepare_cached("DELETE FROM responses WHERE message_id = ?1")?
            .execute(params![id.as_str()])?;
        if let Some(answer) = answer {
            self.connection
                .prepare_cached(
                    "INSERT INTO responses \
                 (message_id, action_id, action_kind, value, answered_via, answered_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )?
                .execute(params![
                    id.as_str(),
                    answer.action_id,
                    enum_text(&answer.action_kind),
                    answer.value,
                    answer.answered_via.as_ref().map(enum_text),
                    to_nanos(answer.answered_at)?,
                ])?;
        }
        Ok(())
    }

    fn select_messages(&self, tail: &str, values: Vec<Value>) -> Result<Vec<Message>, StoreError> {
        let sql = format!("{SELECT_MESSAGE}{tail}");
        let mut statement = self.connection.prepare_cached(&sql)?;
        let mut rows = statement.query(params_from_iter(values))?;
        let mut messages = Vec::new();
        while let Some(row) = rows.next()? {
            let mut message = decode_message(row)?;
            message.actions = self.load_actions(&message.id)?;
            messages.push(message);
        }
        Ok(messages)
    }

    fn load_actions(&self, id: &MessageId) -> Result<Vec<Action>, StoreError> {
        let mut statement = self.connection.prepare_cached(
            "SELECT action_id, label, kind, value, placeholder, destructive \
             FROM actions WHERE message_id = ?1 ORDER BY position",
        )?;
        let mut rows = statement.query(params![id.as_str()])?;
        let mut actions = Vec::new();
        while let Some(row) = rows.next()? {
            actions.push(Action {
                id: row.get(0)?,
                label: row.get(1)?,
                kind: parse_enum::<ActionKind>("kind", &row.get::<_, String>(2)?)?,
                value: row.get(3)?,
                placeholder: row.get(4)?,
                destructive: row.get(5)?,
            });
        }
        Ok(actions)
    }

    fn select_ids(&self, sql: &str, instant: i64) -> Result<Vec<MessageId>, StoreError> {
        let mut statement = self.connection.prepare_cached(sql)?;
        let mut rows = statement.query(params![instant])?;
        let mut ids = Vec::new();
        while let Some(row) = rows.next()? {
            ids.push(MessageId::new(row.get::<_, String>(0)?));
        }
        Ok(ids)
    }
}

fn resolution_kind(resolution: Option<ResolutionReason>) -> Option<&'static str> {
    resolution.map(|resolution| match resolution {
        ResolutionReason::Answered => "answered",
        ResolutionReason::Dismissed => "dismissed",
        ResolutionReason::Rescinded(_) => "rescinded",
        ResolutionReason::Expired => "expired",
    })
}

fn rescind_reason(resolution: Option<ResolutionReason>) -> Option<String> {
    match resolution {
        Some(ResolutionReason::Rescinded(reason)) => Some(enum_text(&reason)),
        _ => None,
    }
}

fn decode_resolution(
    kind: Option<&str>,
    reason: Option<&str>,
) -> Result<Option<ResolutionReason>, StoreError> {
    match (kind, reason) {
        (None, None) => Ok(None),
        (Some("answered"), None) => Ok(Some(ResolutionReason::Answered)),
        (Some("dismissed"), None) => Ok(Some(ResolutionReason::Dismissed)),
        (Some("expired"), None) => Ok(Some(ResolutionReason::Expired)),
        (Some("rescinded"), Some(reason)) => Ok(Some(ResolutionReason::Rescinded(parse_enum::<
            RescindReason,
        >(
            "rescind_reason",
            reason,
        )?))),
        other => Err(StoreError::corrupt("resolution", format!("{other:?}"))),
    }
}

fn decode_message(row: &Row<'_>) -> Result<Message, StoreError> {
    let source = Source {
        agent: row.get("source_agent")?,
        display_name: row.get("source_display_name")?,
        version: row.get("source_version")?,
        host: row.get("source_host")?,
    };
    let conversation = Conversation {
        id: row.get("conversation_id")?,
        title: row.get("conversation_title")?,
        cwd: row.get("conversation_cwd")?,
        focus: from_optional_json::<FocusTarget>(
            "focus",
            row.get::<_, Option<String>>("conversation_focus")?
                .as_deref(),
        )?,
    };
    let answer = match row.get::<_, Option<String>>("answer_action_id")? {
        None => None,
        Some(action_id) => Some(RecordedAnswer {
            action_id,
            action_kind: parse_enum("action_kind", &row.get::<_, String>("answer_action_kind")?)?,
            value: row.get("answer_value")?,
            answered_via: parse_optional_enum::<AnsweredVia>(
                "answered_via",
                row.get::<_, Option<String>>("answer_via")?.as_deref(),
            )?,
            answered_at: from_nanos("answered_at", row.get("answer_at")?)?,
        }),
    };
    let lifecycle = Lifecycle {
        state: parse_enum("state", &row.get::<_, String>("state")?)?,
        snoozed_until: from_optional_nanos("snoozed_until", row.get("snoozed_until")?)?,
        deferred_at: from_optional_nanos("deferred_at", row.get("deferred_at")?)?,
        resolution: decode_resolution(
            row.get::<_, Option<String>>("resolution")?.as_deref(),
            row.get::<_, Option<String>>("rescind_reason")?.as_deref(),
        )?,
        answer,
    };
    let steps: Vec<ActionStep> = from_json("steps", &row.get::<_, String>("steps")?)?;
    let metadata: Metadata = from_json("metadata", &row.get::<_, String>("metadata")?)?;
    let field_sources: FieldSources =
        from_json("field_sources", &row.get::<_, String>("field_sources")?)?;
    Ok(Message {
        id: MessageId::new(row.get::<_, String>("id")?),
        idempotency_key: row.get("idempotency_key")?,
        source,
        conversation,
        topic_id: TopicId::new(row.get::<_, String>("topic_id")?),
        thread_id: ThreadId::new(row.get::<_, String>("thread_id")?),
        message_type: parse_enum("type", &row.get::<_, String>("type")?)?,
        urgent: row.get("urgent")?,
        blocking: parse_enum("blocking", &row.get::<_, String>("blocking")?)?,
        field_sources,
        muted_by_rule: row.get("muted_by_rule")?,
        event: row.get("event")?,
        title: row.get("title")?,
        summary: row.get("summary")?,
        body: row.get("body")?,
        steps,
        steps_completed: from_json("steps_completed", &row.get::<_, String>("steps_completed")?)?,
        actions: Vec::new(),
        scope: parse_optional_enum::<MessageScope>(
            "scope",
            row.get::<_, Option<String>>("scope")?.as_deref(),
        )?,
        awaits_response: row.get("awaits_response")?,
        correlation_key: row.get("correlation_key")?,
        sensitive: row.get("sensitive")?,
        created_at: from_optional_nanos("created_at", row.get("created_at")?)?,
        received_at: from_nanos("received_at", row.get("received_at")?)?,
        expires_at: from_nanos("expires_at", row.get("expires_at")?)?,
        metadata,
        lifecycle,
        repeat_count: row.get("repeat_count")?,
    })
}

const SELECT_MESSAGE: &str = "\
SELECT m.id, m.idempotency_key, m.source_agent, m.source_display_name, m.source_version, \
m.source_host, m.conversation_id, m.topic_id, m.thread_id, m.type, m.urgent, m.blocking, \
m.field_sources, m.muted_by_rule, m.event, m.title, m.summary, m.body, m.steps, \
m.steps_completed, m.scope, m.awaits_response, m.correlation_key, m.sensitive, m.created_at, \
m.received_at, m.expires_at, m.metadata, m.repeat_count, m.state, m.snoozed_until, \
m.deferred_at, m.resolution, m.rescind_reason, \
c.title AS conversation_title, c.cwd AS conversation_cwd, c.focus AS conversation_focus, \
r.action_id AS answer_action_id, r.action_kind AS answer_action_kind, r.value AS answer_value, \
r.answered_via AS answer_via, r.answered_at AS answer_at \
FROM messages m \
LEFT JOIN conversations c ON c.agent = m.source_agent AND c.conversation_id = m.conversation_id \
LEFT JOIN responses r ON r.message_id = m.id";

const INSERT_MESSAGE: &str = "\
INSERT INTO messages (id, idempotency_key, source_agent, source_display_name, source_version, \
source_host, conversation_id, topic_id, thread_id, type, urgent, blocking, field_sources, \
muted_by_rule, event, title, summary, body, steps, steps_completed, scope, awaits_response, \
correlation_key, sensitive, created_at, received_at, expires_at, metadata, repeat_count, state, \
snoozed_until, deferred_at, resolution, rescind_reason, closed_at, updated_at) \
VALUES (:id, :idempotency_key, :source_agent, :source_display_name, :source_version, \
:source_host, :conversation_id, :topic_id, :thread_id, :type, :urgent, :blocking, \
:field_sources, :muted_by_rule, :event, :title, :summary, :body, :steps, :steps_completed, \
:scope, :awaits_response, :correlation_key, :sensitive, :created_at, :received_at, \
:expires_at, :metadata, :repeat_count, :state, :snoozed_until, :deferred_at, :resolution, \
:rescind_reason, :closed_at, :updated_at)";

const UPDATE_MESSAGE: &str = "\
UPDATE messages SET type = :type, urgent = :urgent, blocking = :blocking, \
field_sources = :field_sources, muted_by_rule = :muted_by_rule, event = :event, \
title = :title, summary = :summary, body = :body, steps = :steps, \
steps_completed = :steps_completed, scope = :scope, awaits_response = :awaits_response, \
correlation_key = :correlation_key, sensitive = :sensitive, created_at = :created_at, \
expires_at = :expires_at, metadata = :metadata, repeat_count = :repeat_count, state = :state, \
snoozed_until = :snoozed_until, deferred_at = :deferred_at, resolution = :resolution, \
rescind_reason = :rescind_reason, \
closed_at = CASE WHEN :is_closed THEN COALESCE(closed_at, :now) ELSE NULL END, \
updated_at = :now \
WHERE id = :id";

const UPSERT_CONVERSATION: &str = "\
INSERT INTO conversations (agent, conversation_id, title, cwd, focus, updated_at) \
VALUES (:agent, :conversation_id, :title, :cwd, :focus, :updated_at) \
ON CONFLICT (agent, conversation_id) DO UPDATE SET \
title = COALESCE(excluded.title, title), \
cwd = COALESCE(excluded.cwd, cwd), \
focus = COALESCE(excluded.focus, focus), \
updated_at = excluded.updated_at";

/// A `WHERE` clause built from bound parameters only: no value is ever
/// spliced into the SQL text.
#[derive(Default)]
struct WhereClause {
    conditions: Vec<String>,
    values: Vec<Value>,
}

impl WhereClause {
    fn bind(&mut self, value: Value) -> String {
        self.values.push(value);
        format!("?{}", self.values.len())
    }

    fn push_equal(&mut self, column: &str, value: Value) {
        let placeholder = self.bind(value);
        self.conditions.push(format!("{column} = {placeholder}"));
    }

    fn push_any_of(&mut self, column: &str, values: Vec<Value>) {
        let placeholders: Vec<String> = values.into_iter().map(|value| self.bind(value)).collect();
        self.conditions
            .push(format!("{column} IN ({})", placeholders.join(", ")));
    }

    fn push_snoozed_before(&mut self, nanos: i64) {
        let placeholder = self.bind(Value::Integer(nanos));
        self.conditions.push(format!(
            "(m.snoozed_until IS NULL OR m.snoozed_until <= {placeholder})"
        ));
    }

    fn render(&self) -> String {
        if self.conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", self.conditions.join(" AND "))
        }
    }
}
