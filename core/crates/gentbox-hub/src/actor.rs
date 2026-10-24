//! The hub actor: one task that owns the store and the in-memory view (live
//! waiters, what each surface shows, timers) and applies every command in
//! turn (architecture §4).
//!
//! Each change is written in one store transaction (`BEGIN IMMEDIATE`), and
//! only after it commits does the actor wake waiters and tell the surfaces.

use std::collections::{BTreeMap, HashMap};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Duration as StdDuration;

use gentbox_core::action_plan::set_step_ticked;
use gentbox_core::actions::offered_actions;
use gentbox_core::classify::{BuiltInRules, RuleSet};
use gentbox_core::coalesce::{CoalesceInput, CoalesceOutcome, coalesce, progress_rescissions};
use gentbox_core::ids::{ConversationKey, MessageId, ThreadId, TopicId};
use gentbox_core::item::{DeliveredSurfaces, OpenItem};
use gentbox_core::lifecycle::{
    ActionOutcome, ActionPress, LifecycleEvent, LifecycleState, apply_event, dismiss, press_action,
};
use gentbox_core::message::{Action, ActionKind, Conversation, Message, MessageInput};
use gentbox_core::policy::{
    DeliveryPlan, MacSurface, Policy, PolicyContext, Presence, PresenceState, decide,
};
use gentbox_core::prepare::{IngestSettings, prepare_message};
use gentbox_core::rescind::{
    RescindError, RescindReason, RescindRequest, Rescission, rescind_by_correlation_keys,
    rescind_by_id,
};
use gentbox_core::resolve::{
    GroupingError, GroupingRequest, GroupingSnapshot, NewThread, NewTopic, Resolution,
    ThreadCreateAction, ThreadCreateContext, ThreadTarget, TopicCreateOutcome, TopicTarget,
    create_thread, create_topic, resolve_grouping,
};
use gentbox_core::response::{ActionResponse, IngestResponse, IngestWarning, WarningCode};
use gentbox_core::secret_scan::{redact_optional, redact_secrets};
use gentbox_core::title_guard::{guard_conversation_title, is_plain_words};
use gentbox_core::topic::{
    CloseRequest, CreatedBy, GroupStatus, Thread, ThreadCreate, ThreadRecord, ThreadRef, Topic,
    TopicCreate, TopicRecord, TopicRef, is_effectively_muted,
};
use gentbox_store::{
    DeliveryEvent, DeliveryRecord, MessageFilter, Repository, RetentionPolicy, Store, StoreError,
    SweepReport, Upserted,
};
use time::{Duration, OffsetDateTime};
use tokio::sync::{mpsc, watch};

use crate::clock::Clock;
use crate::commands::{
    ActionRequest, ConversationUpdate, HubError, IngestOutcome, IngestStatus, Job, MessageGrouping,
    MessageThreadMove, Requester, ThreadMove, ThreadUpdate, TopicUpdate,
};
use crate::config::source_settings;
use crate::dispatch::{Dispatcher, HubEvent};
use crate::lock::HubLock;
use crate::timers::{TimerKind, Timers};
use crate::waiters::{WaitStart, WaiterSignal, Waiters};

/// How often the retention sweep runs (it also runs at start).
pub(crate) const SWEEP_INTERVAL: Duration = Duration::days(1);
/// The longest the actor sleeps without looking at the clock, so a system
/// sleep or a clock change never delays a timer for long.
const MAX_IDLE: StdDuration = StdDuration::from_secs(30);
/// Longest reply text a sender is handed (`actionResponse.value`).
const MAX_REPLY_CHARS: usize = 8192;

/// What a surface shows for a live message, and its re-pings so far.
#[derive(Debug, Clone)]
struct Delivery {
    plan: DeliveryPlan,
    surfaces: DeliveredSurfaces,
    repings_sent: u8,
}

/// The actor's state. Only the actor task touches it.
pub(crate) struct HubState {
    store: Store,
    clock: Arc<dyn Clock>,
    rules: RuleSet,
    policy: Policy,
    sources: BTreeMap<String, IngestSettings>,
    retention: RetentionPolicy,
    has_answer_channel: bool,
    presence: Presence,
    is_focused: bool,
    is_quiet_hours: bool,
    waiters: Waiters,
    deliveries: HashMap<MessageId, Delivery>,
    timers: Timers,
    dispatcher: Dispatcher,
    is_stopping: bool,
    // Declared last so it drops last: the lock outlives the store.
    _lock: HubLock,
}

/// What [`HubState::new`] needs.
pub(crate) struct StateParts {
    pub(crate) store: Store,
    pub(crate) lock: HubLock,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) rules: RuleSet,
    pub(crate) policy: Policy,
    pub(crate) sources: BTreeMap<String, IngestSettings>,
    pub(crate) retention: RetentionPolicy,
    pub(crate) has_answer_channel: bool,
    pub(crate) dispatcher: Dispatcher,
    pub(crate) signals: mpsc::UnboundedSender<WaiterSignal>,
}

/// Runs the actor until it is told to stop or every handle is gone. It runs
/// on the hub's own thread, so SQLite's commits and the sweep never block a
/// runtime that serves HTTP.
pub(crate) async fn run(
    mut state: HubState,
    mut jobs: mpsc::Receiver<Job>,
    mut signals: mpsc::UnboundedReceiver<WaiterSignal>,
) {
    loop {
        state.fire_due_timers();
        if state.is_stopping {
            break;
        }
        let idle = state.idle_time();
        tokio::select! {
            biased;
            Some(signal) = signals.recv() => guarded("a waiter signal", || state.on_signal(signal)),
            job = jobs.recv() => match job {
                Some(job) => {
                    state.fire_due_timers();
                    guarded("a hub command", || job(&mut state));
                }
                None => break,
            },
            () = tokio::time::sleep(idle) => {}
        }
    }
    // Close first, so callers whose queued commands are dropped here see the
    // hub as stopped, not as a command that failed.
    jobs.close();
    while jobs.try_recv().is_ok() {}
    tracing::info!("hub stopped");
}

/// Runs one step of the actor; a panic is logged and the actor keeps going.
fn guarded(what: &str, step: impl FnOnce()) {
    if catch_unwind(AssertUnwindSafe(step)).is_err() {
        tracing::error!(
            "{what} panicked; a store transaction still open was rolled back, but \
             the hub's in-memory view may lag the store until it restarts"
        );
    }
}

/// Reports the actor's end, even when it unwinds. Created first on the hub
/// thread, so it drops last: after the store has closed and the lock is
/// released.
pub(crate) struct StoppedOnDrop(pub(crate) watch::Sender<bool>);

impl Drop for StoppedOnDrop {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}

impl HubState {
    pub(crate) fn new(parts: StateParts) -> Result<Self, StoreError> {
        let mut state = Self {
            store: parts.store,
            clock: parts.clock,
            rules: parts.rules,
            policy: parts.policy,
            sources: parts.sources,
            retention: parts.retention,
            has_answer_channel: parts.has_answer_channel,
            presence: Presence {
                state: PresenceState::Present,
                frontmost_bundle_id: None,
            },
            is_focused: false,
            is_quiet_hours: false,
            waiters: Waiters::new(parts.signals),
            deliveries: HashMap::new(),
            timers: Timers::default(),
            dispatcher: parts.dispatcher,
            is_stopping: false,
            _lock: parts.lock,
        };
        state.restore()?;
        Ok(state)
    }

    /// After a start: expiry and snooze timers for every live message, and a
    /// sweep. Nothing is on screen yet, so nothing re-pings; the UI lists the
    /// live messages itself.
    fn restore(&mut self) -> Result<(), StoreError> {
        let now = self.now();
        for message in self
            .store
            .repository()
            .list_messages(&MessageFilter::default())?
        {
            self.timers
                .schedule(message.expires_at, TimerKind::Expiry(message.id.clone()));
            if let Some(until) = message.lifecycle.snoozed_until {
                self.timers
                    .schedule(until, TimerKind::SnoozeEnd(message.id.clone()));
            }
        }
        self.timers.schedule(now, TimerKind::Sweep);
        Ok(())
    }

    pub(crate) fn store(&self) -> Repository<'_> {
        self.store.repository()
    }

    pub(crate) fn stop(&mut self) {
        self.is_stopping = true;
    }

    fn now(&self) -> OffsetDateTime {
        self.clock.now()
    }

    fn idle_time(&self) -> StdDuration {
        let Some(deadline) = self.timers.next_deadline() else {
            return MAX_IDLE;
        };
        let until = deadline - self.now();
        StdDuration::try_from(until)
            .unwrap_or(StdDuration::ZERO)
            .min(MAX_IDLE)
    }

    // -----------------------------------------------------------------
    // Ingest
    // -----------------------------------------------------------------

    pub(crate) fn ingest(&mut self, input: MessageInput) -> Result<IngestOutcome, HubError> {
        let now = self.now();
        if let Some(stored) = self.idempotent_match(&input)? {
            let should_wait = self.current_should_wait(&stored)?;
            return Ok(IngestOutcome {
                status: IngestStatus::Existing,
                response: ingest_response(&stored, should_wait, Vec::new()),
            });
        }
        let settings = source_settings(&self.sources, &input.source.agent);
        let prepared = prepare_message(input, &self.rules, BuiltInRules::load(), settings)
            .map_err(HubError::Rejected)?;
        let rescinds = prepared.message.rescinds.clone();
        let mut warnings = prepared.warnings.clone();
        let snapshot = self.store.repository().grouping_snapshot()?;
        let resolution = resolve_grouping(&snapshot, &prepared.grouping_request())
            .map_err(HubError::Grouping)?;
        let created_by = CreatedBy {
            agent: prepared.message.source.agent.clone(),
            conversation_id: Some(prepared.message.conversation.id.clone()),
        };
        let groups = PlannedGroups::new(&snapshot, resolution, &created_by, now);
        let message = prepared.into_message(
            new_message_id(now),
            now,
            groups.topic_id.clone(),
            groups.thread_id.clone(),
        );
        let plan = self.plan_for(&message, groups.is_muted);
        warnings.extend(muted_warning(&message, &groups));
        let items = self.open_items(&message.conversation_key(), now)?;
        let outcome = coalesce(&CoalesceInput {
            incoming: &message,
            rescinds: rescinds.as_ref(),
            idempotent_match: None,
            open_items: &items,
            will_have_live_waiter: plan.should_wait,
            has_phone_channel: self.dispatcher.has_phone_channel(),
        });
        match outcome {
            CoalesceOutcome::Duplicate {
                of, rescissions, ..
            } => self.absorb_repeat(&of, rescissions, plan.should_wait, warnings, now),
            CoalesceOutcome::Folded { into, rescissions } => {
                self.absorb_repeat(&into, rescissions, false, warnings, now)
            }
            CoalesceOutcome::Superseded {
                replaces,
                pings,
                rescissions,
            } => self.store_new(
                message,
                plan,
                &groups,
                Some(Supersede { replaces, pings }),
                rescissions,
                warnings,
                now,
            ),
            CoalesceOutcome::New { rescissions } => {
                self.store_new(message, plan, &groups, None, rescissions, warnings, now)
            }
        }
    }

    fn idempotent_match(&self, input: &MessageInput) -> Result<Option<Message>, HubError> {
        let Some(key) = input.idempotency_key.as_deref() else {
            return Ok(None);
        };
        Ok(self
            .store
            .repository()
            .find_message_by_idempotency_key(&input.source.agent, key)?)
    }

    #[allow(clippy::too_many_arguments)]
    fn store_new(
        &mut self,
        message: Message,
        plan: DeliveryPlan,
        groups: &PlannedGroups,
        supersede: Option<Supersede>,
        rescissions: Vec<Rescission>,
        warnings: Vec<IngestWarning>,
        now: OffsetDateTime,
    ) -> Result<IngestOutcome, HubError> {
        let closed = self.store.transaction(|repo| {
            groups.write(repo, &message, now)?;
            repo.insert_message(&message, now)?;
            if let Some(supersede) = &supersede {
                repo.record_supersession(&supersede.replaces, &message.id)?;
            }
            log_delivery(repo, &message, &plan, &warnings, now)?;
            close_withdrawn(repo, &rescissions, now)
        })?;

        let replaced = supersede
            .as_ref()
            .and_then(|supersede| self.deliveries.remove(&supersede.replaces));
        let repings_sent = match (&supersede, replaced) {
            (Some(supersede), Some(old)) if !supersede.pings => old.repings_sent,
            _ => 0,
        };
        self.track_delivery(&message, &plan, repings_sent, now);
        if plan.should_wait {
            let lapse_at = self.waiters.expect(&message.id, now);
            self.timers
                .schedule(lapse_at, TimerKind::WaiterLapse(message.id.clone()));
        }
        self.dispatcher.send(HubEvent::Delivered {
            offered_actions: cloned(offered_actions(&message, plan.should_wait)),
            replaces: supersede
                .as_ref()
                .map(|supersede| supersede.replaces.clone()),
            pings: supersede.as_ref().is_none_or(|supersede| supersede.pings),
            plan: plan.clone(),
            message: message.clone(),
        });
        self.announce_groups(groups);
        self.finish_withdrawals(closed, now);
        Ok(IngestOutcome {
            status: IngestStatus::Created,
            response: ingest_response(&message, plan.should_wait, warnings),
        })
    }

    /// A fold or a dedupe: the open message counts one more repeat; the
    /// incoming message's own `rescinds` still apply.
    fn absorb_repeat(
        &mut self,
        target: &MessageId,
        rescissions: Vec<Rescission>,
        should_wait: bool,
        warnings: Vec<IngestWarning>,
        now: OffsetDateTime,
    ) -> Result<IngestOutcome, HubError> {
        let (absorbing, closed) = self.store.transaction(|repo| {
            let mut absorbing = repo.message(target)?;
            absorbing.repeat_count = absorbing.repeat_count.saturating_add(1);
            repo.update_message(&absorbing, now)?;
            let closed = close_withdrawn(repo, &rescissions, now)?;
            Ok((absorbing, closed))
        })?;
        self.dispatcher.send(HubEvent::Repeated {
            message_id: target.clone(),
            repeat_count: absorbing.repeat_count,
        });
        if should_wait {
            // Only a new message's Delivered event carries the buttons; here
            // the UI must hear that a sender now waits.
            let lapse_at = self.waiters.polled(target, now);
            self.timers
                .schedule(lapse_at, TimerKind::WaiterLapse(target.clone()));
            self.announce_waiter(&absorbing, now);
        }
        self.finish_withdrawals(closed, now);
        Ok(IngestOutcome {
            status: IngestStatus::Existing,
            response: ingest_response(&absorbing, should_wait, warnings),
        })
    }

    fn track_delivery(
        &mut self,
        message: &Message,
        plan: &DeliveryPlan,
        repings_sent: u8,
        now: OffsetDateTime,
    ) {
        self.timers
            .schedule(message.expires_at, TimerKind::Expiry(message.id.clone()));
        if let Some(reping) = plan.reping
            && message.lifecycle.state == LifecycleState::Open
            && repings_sent < reping.max
        {
            self.timers
                .schedule(now + reping.after, TimerKind::Reping(message.id.clone()));
        }
        self.deliveries.insert(
            message.id.clone(),
            Delivery {
                plan: plan.clone(),
                surfaces: DeliveredSurfaces::from_plan(plan),
                repings_sent,
            },
        );
    }

    // -----------------------------------------------------------------
    // Long-poll
    // -----------------------------------------------------------------

    pub(crate) fn begin_wait(&mut self, id: &MessageId) -> Result<WaitStart, HubError> {
        let now = self.now();
        let Some(message) = self.store.repository().find_message(id)? else {
            if self.store.repository().was_swept(id)? {
                return Ok(WaitStart::Ready(ActionResponse::gone(id.clone())));
            }
            return Err(HubError::MessageNotFound(id.clone()));
        };
        let response = ActionResponse::for_message(&message, self.waiters.deferral_unreported(id));
        use gentbox_core::response::ResponseStatus;
        let start = match response.status {
            ResponseStatus::Pending => WaitStart::Waiting(self.waiters.begin(response)),
            ResponseStatus::Deferred => {
                self.waiters.deferral_reported(id);
                let lapse_at = self.waiters.polled(id, now);
                self.timers
                    .schedule(lapse_at, TimerKind::WaiterLapse(id.clone()));
                WaitStart::Ready(response)
            }
            _ => return Ok(WaitStart::Ready(response)),
        };
        self.announce_waiter(&message, now);
        Ok(start)
    }

    pub(crate) fn on_signal(&mut self, signal: WaiterSignal) {
        let now = self.now();
        match signal {
            WaiterSignal::Ended {
                message_id,
                reported_deferral,
            } => {
                if let Some(lapse_at) = self.waiters.end(&message_id, now, reported_deferral) {
                    self.timers
                        .schedule(lapse_at, TimerKind::WaiterLapse(message_id));
                }
            }
        }
    }

    pub(crate) fn polls_in_flight(&self, id: &MessageId) -> u32 {
        self.waiters.in_flight(id)
    }

    pub(crate) fn tracked_waiters(&self) -> usize {
        self.waiters.len()
    }

    pub(crate) fn has_live_waiter(&self, id: &MessageId) -> bool {
        self.waiters.is_live(id, self.now())
    }

    /// Tells the UI when the live waiter appeared or lapsed.
    fn announce_waiter(&mut self, message: &Message, now: OffsetDateTime) {
        if let Some(has_live_waiter) = self.waiters.take_live_change(&message.id, now) {
            self.dispatcher.send(HubEvent::WaiterChanged {
                message_id: message.id.clone(),
                has_live_waiter,
                offered_actions: cloned(offered_actions(message, has_live_waiter)),
            });
        }
    }

    // -----------------------------------------------------------------
    // Rescind and progress
    // -----------------------------------------------------------------

    pub(crate) fn rescind(
        &mut self,
        id: &MessageId,
        reason: RescindReason,
        requester: &Requester,
    ) -> Result<ActionResponse, HubError> {
        let now = self.now();
        let message = self.store.repository().message(id)?;
        if !requester.owns(&message) {
            return Err(HubError::NotOwner);
        }
        let item = self.item_of(&message, now);
        let rescission = rescind_by_id(&item, reason, self.dispatcher.has_phone_channel())
            .map_err(|_| HubError::MessageClosed {
                state: message.lifecycle.state,
                resolution: message.lifecycle.resolution,
            })?;
        let closed = self.withdraw(vec![rescission], now)?;
        let message = closed.first().unwrap_or(&message);
        Ok(ActionResponse::for_message(message, false))
    }

    pub(crate) fn rescind_by_correlation_keys(
        &mut self,
        conversation: &ConversationKey,
        request: &RescindRequest,
    ) -> Result<Vec<MessageId>, HubError> {
        let now = self.now();
        let items = self.open_items(conversation, now)?;
        let rescissions = rescind_by_correlation_keys(
            conversation,
            request,
            &items,
            self.dispatcher.has_phone_channel(),
        )
        .map_err(|error| match error {
            RescindError::MissingCorrelationKeys => HubError::MissingCorrelationKeys,
            RescindError::AlreadyClosed { state } => HubError::MessageClosed {
                state,
                resolution: None,
            },
        })?;
        Ok(ids_of(&self.withdraw(rescissions, now)?))
    }

    pub(crate) fn progress(
        &mut self,
        conversation: &ConversationKey,
    ) -> Result<Vec<MessageId>, HubError> {
        let now = self.now();
        let items = self.open_items(conversation, now)?;
        let rescissions =
            progress_rescissions(conversation, &items, self.dispatcher.has_phone_channel());
        Ok(ids_of(&self.withdraw(rescissions, now)?))
    }

    /// Withdraws messages in one transaction, then wakes their waiters and
    /// tells the surfaces.
    fn withdraw(
        &mut self,
        rescissions: Vec<Rescission>,
        now: OffsetDateTime,
    ) -> Result<Vec<Message>, HubError> {
        if rescissions.is_empty() {
            return Ok(Vec::new());
        }
        let closed = self
            .store
            .transaction(|repo| close_withdrawn(repo, &rescissions, now))?;
        let messages = closed.iter().map(|(message, _)| message.clone()).collect();
        self.finish_withdrawals(closed, now);
        Ok(messages)
    }

    fn finish_withdrawals(&mut self, closed: Vec<(Message, Rescission)>, now: OffsetDateTime) {
        for (message, rescission) in closed {
            self.after_close(&message, now);
            self.dispatcher.send(HubEvent::Withdrawn(rescission));
        }
    }

    /// A message reached a terminal state: its waiter hears it, its timers
    /// and its surfaces go. Returns what was on screen.
    fn after_close(&mut self, message: &Message, now: OffsetDateTime) -> DeliveredSurfaces {
        self.waiters
            .publish(ActionResponse::for_message(message, false));
        self.waiters.forget_if_idle(&message.id, now);
        self.timers.cancel_message(&message.id);
        self.deliveries
            .remove(&message.id)
            .map(|delivery| delivery.surfaces)
            .unwrap_or_default()
    }

    fn withdraw_group_messages(
        &mut self,
        filter: MessageFilter,
        reason: RescindReason,
        requester: Option<&Requester>,
    ) -> Result<Vec<MessageId>, HubError> {
        let now = self.now();
        let rescissions = self.group_rescissions(&filter, reason, requester, now)?;
        Ok(ids_of(&self.withdraw(rescissions, now)?))
    }

    /// The rescissions that withdraw a topic's or thread's live messages: the
    /// requester's only, or all of them for the UI.
    fn group_rescissions(
        &self,
        filter: &MessageFilter,
        reason: RescindReason,
        requester: Option<&Requester>,
        now: OffsetDateTime,
    ) -> Result<Vec<Rescission>, HubError> {
        let messages = self.store.repository().list_messages(filter)?;
        Ok(messages
            .iter()
            .filter(|message| requester.is_none_or(|requester| requester.owns(message)))
            .filter_map(|message| {
                let item = self.item_of(message, now);
                rescind_by_id(&item, reason, self.dispatcher.has_phone_channel()).ok()
            })
            .collect())
    }

    /// The withdrawals a close asks for, if any.
    fn close_rescissions(
        &self,
        filter: MessageFilter,
        request: &CloseRequest,
        requester: Option<&Requester>,
        now: OffsetDateTime,
    ) -> Result<Vec<Rescission>, HubError> {
        if !request.rescind_open_messages {
            return Ok(Vec::new());
        }
        self.group_rescissions(&filter, request.rescind_reason(), requester, now)
    }

    // -----------------------------------------------------------------
    // Conversations
    // -----------------------------------------------------------------

    pub(crate) fn upsert_conversation(
        &mut self,
        key: &ConversationKey,
        mut conversation: Conversation,
    ) -> Result<ConversationUpdate, HubError> {
        let now = self.now();
        let mut warnings = Vec::new();
        let (title, redactions) = redact_optional(conversation.title.as_deref());
        if redactions > 0 {
            warnings.push(IngestWarning::new(
                WarningCode::SecretRedacted,
                "title",
                format!("{redactions} secret(s) in the title were replaced by [redacted]; never put secrets in a title."),
            ));
        }
        let (title, ignored) = guard_conversation_title(title.as_deref());
        warnings.extend(ignored);
        conversation.title = title;
        let renamed = self.store.transaction(|repo| {
            repo.upsert_conversation(key, &conversation, now)?;
            match &conversation.title {
                Some(title) => rename_implicit_threads(repo, key, title, now),
                None => Ok(Vec::new()),
            }
        })?;
        for thread_id in renamed {
            self.dispatcher.send(HubEvent::ThreadChanged(thread_id));
        }
        Ok(ConversationUpdate {
            conversation: self.store.repository().conversation(key)?,
            warnings,
        })
    }

    // -----------------------------------------------------------------
    // Topics
    // -----------------------------------------------------------------

    pub(crate) fn create_topic(
        &mut self,
        agent: &str,
        request: &TopicCreate,
    ) -> Result<Upserted<TopicRecord>, HubError> {
        let now = self.now();
        let snapshot = self.store.repository().grouping_snapshot()?;
        match create_topic(&snapshot, request).map_err(HubError::Grouping)? {
            TopicCreateOutcome::Existing(id) => Ok(Upserted::Existing(
                self.store.repository().topic_record(&id)?,
            )),
            TopicCreateOutcome::Created(new) => {
                let created_by = CreatedBy {
                    agent: agent.to_owned(),
                    conversation_id: request
                        .conversation
                        .as_ref()
                        .map(|conversation| conversation.id.clone()),
                };
                let topic = topic_from(new, new_group_id(now), &created_by, now);
                self.store.repository().insert_topic(&topic)?;
                self.dispatcher
                    .send(HubEvent::TopicChanged(topic.id.clone()));
                Ok(Upserted::Created(
                    self.store.repository().topic_record(&topic.id)?,
                ))
            }
        }
    }

    pub(crate) fn update_topic(
        &mut self,
        id: &TopicId,
        update: TopicUpdate,
    ) -> Result<TopicRecord, HubError> {
        let now = self.now();
        let mut topic = self.store.repository().topic(id)?;
        if topic.implicit {
            return Err(HubError::ImplicitGroup);
        }
        if let Some(title) = update.title {
            topic.title = guarded_group_title(&title)?;
        }
        if let Some(summary) = update.summary {
            topic.summary = redact_optional(summary.as_deref()).0;
        }
        if let Some(style) = update.style {
            topic.style = Some(style);
        }
        if let Some(kind) = update.kind {
            topic.kind = Some(kind);
        }
        topic.updated_at = now;
        self.store.repository().update_topic(&topic)?;
        self.dispatcher.send(HubEvent::TopicChanged(id.clone()));
        Ok(self.store.repository().topic_record(id)?)
    }

    pub(crate) fn close_topic(
        &mut self,
        id: &TopicId,
        request: &CloseRequest,
        requester: Option<&Requester>,
    ) -> Result<TopicRecord, HubError> {
        let now = self.now();
        let filter = MessageFilter {
            topic_id: Some(id.clone()),
            ..MessageFilter::default()
        };
        let rescissions = self.close_rescissions(filter, request, requester, now)?;
        // The close and its withdrawals commit together or not at all.
        let (closed_threads, withdrawn) = self.store.transaction(|repo| {
            let closed_threads = repo.close_topic(id, now)?;
            Ok((closed_threads, close_withdrawn(repo, &rescissions, now)?))
        })?;
        self.finish_withdrawals(withdrawn, now);
        self.dispatcher.send(HubEvent::TopicChanged(id.clone()));
        for thread_id in closed_threads {
            self.dispatcher.send(HubEvent::ThreadChanged(thread_id));
        }
        Ok(self.store.repository().topic_record(id)?)
    }

    pub(crate) fn reopen_topic(&mut self, id: &TopicId) -> Result<TopicRecord, HubError> {
        let reopened = self.store.repository().reopen_topic(id, self.now())?;
        self.dispatcher.send(HubEvent::TopicChanged(id.clone()));
        for thread_id in reopened {
            self.dispatcher.send(HubEvent::ThreadChanged(thread_id));
        }
        Ok(self.store.repository().topic_record(id)?)
    }

    pub(crate) fn rescind_in_topic(
        &mut self,
        id: &TopicId,
        reason: RescindReason,
        requester: Option<&Requester>,
    ) -> Result<Vec<MessageId>, HubError> {
        self.store.repository().topic(id)?;
        self.withdraw_group_messages(
            MessageFilter {
                topic_id: Some(id.clone()),
                ..MessageFilter::default()
            },
            reason,
            requester,
        )
    }

    pub(crate) fn mute_topic(
        &mut self,
        id: &TopicId,
        is_muted: bool,
    ) -> Result<TopicRecord, HubError> {
        let mut topic = self.store.repository().topic(id)?;
        topic.muted = is_muted;
        topic.updated_at = self.now();
        self.store.repository().update_topic(&topic)?;
        self.dispatcher.send(HubEvent::TopicChanged(id.clone()));
        Ok(self.store.repository().topic_record(id)?)
    }

    pub(crate) fn mark_topic_read(&mut self, id: &TopicId) -> Result<Vec<MessageId>, HubError> {
        let acknowledged = self.store.repository().mark_topic_read(id, self.now())?;
        self.after_mark_read(&acknowledged)?;
        self.dispatcher.send(HubEvent::TopicChanged(id.clone()));
        Ok(acknowledged)
    }

    // -----------------------------------------------------------------
    // Threads
    // -----------------------------------------------------------------

    pub(crate) fn create_thread(
        &mut self,
        agent: &str,
        request: &ThreadCreate,
    ) -> Result<Upserted<ThreadRecord>, HubError> {
        let now = self.now();
        let snapshot = self.store.repository().grouping_snapshot()?;
        // A thread create carries no message for rules to match, so no rule
        // assigns its topic.
        let context = ThreadCreateContext {
            agent,
            rule_topic: None,
        };
        let outcome = create_thread(&snapshot, request, &context).map_err(HubError::Grouping)?;
        let created_by = CreatedBy {
            agent: agent.to_owned(),
            conversation_id: request
                .conversation
                .as_ref()
                .map(|conversation| conversation.id.clone()),
        };
        let (topic_id, new_topic) = match outcome.topic {
            TopicTarget::Existing(id) => (id, None),
            TopicTarget::Create(new) => {
                let topic = topic_from(new, new_group_id(now), &created_by, now);
                (topic.id.clone(), Some(topic))
            }
        };
        let (thread, takeover) = match outcome.action {
            ThreadCreateAction::Existing(id) => {
                return Ok(Upserted::Existing(
                    self.store.repository().thread_record(&id)?,
                ));
            }
            ThreadCreateAction::Created { thread, takeover } => (
                thread_from(thread, new_group_id(now), &topic_id, &created_by, now),
                takeover,
            ),
        };
        self.store.transaction(|repo| {
            if let Some(topic) = &new_topic {
                repo.insert_topic(topic)?;
            }
            match &takeover {
                Some(takeover) => repo
                    .insert_thread_with_takeover(&thread, takeover, now)
                    .map(|_| ()),
                None => repo.insert_thread(&thread),
            }
        })?;
        if new_topic.is_some() {
            self.dispatcher.send(HubEvent::TopicChanged(topic_id));
        }
        self.dispatcher
            .send(HubEvent::ThreadChanged(thread.id.clone()));
        Ok(Upserted::Created(
            self.store.repository().thread_record(&thread.id)?,
        ))
    }

    pub(crate) fn update_thread(
        &mut self,
        id: &ThreadId,
        update: ThreadUpdate,
    ) -> Result<ThreadRecord, HubError> {
        let now = self.now();
        let mut thread = self.store.repository().thread(id)?;
        if thread.implicit {
            return Err(HubError::ImplicitGroup);
        }
        if let Some(title) = update.title {
            thread.title = guarded_group_title(&title)?;
        }
        if let Some(summary) = update.summary {
            thread.summary = redact_optional(summary.as_deref()).0;
        }
        if let Some(kind) = update.kind {
            thread.kind = Some(kind);
        }
        thread.updated_at = now;
        self.store.repository().update_thread(&thread)?;
        self.dispatcher.send(HubEvent::ThreadChanged(id.clone()));
        Ok(self.store.repository().thread_record(id)?)
    }

    pub(crate) fn close_thread(
        &mut self,
        id: &ThreadId,
        request: &CloseRequest,
        requester: Option<&Requester>,
    ) -> Result<ThreadRecord, HubError> {
        let now = self.now();
        let filter = MessageFilter {
            thread_id: Some(id.clone()),
            ..MessageFilter::default()
        };
        let rescissions = self.close_rescissions(filter, request, requester, now)?;
        let withdrawn = self.store.transaction(|repo| {
            repo.close_thread(id, now)?;
            close_withdrawn(repo, &rescissions, now)
        })?;
        self.finish_withdrawals(withdrawn, now);
        self.dispatcher.send(HubEvent::ThreadChanged(id.clone()));
        Ok(self.store.repository().thread_record(id)?)
    }

    pub(crate) fn reopen_thread(&mut self, id: &ThreadId) -> Result<ThreadRecord, HubError> {
        let reopened = self.store.repository().reopen_thread(id, self.now())?;
        let thread = self.store.repository().thread(id)?;
        if reopened.topic {
            self.dispatcher
                .send(HubEvent::TopicChanged(thread.topic_id.clone()));
        }
        self.dispatcher.send(HubEvent::ThreadChanged(id.clone()));
        Ok(self.store.repository().thread_record(id)?)
    }

    pub(crate) fn rescind_in_thread(
        &mut self,
        id: &ThreadId,
        reason: RescindReason,
        requester: Option<&Requester>,
    ) -> Result<Vec<MessageId>, HubError> {
        self.store.repository().thread(id)?;
        self.withdraw_group_messages(
            MessageFilter {
                thread_id: Some(id.clone()),
                ..MessageFilter::default()
            },
            reason,
            requester,
        )
    }

    pub(crate) fn move_thread(
        &mut self,
        id: &ThreadId,
        request: &ThreadMove,
        requester: &Requester,
    ) -> Result<ThreadRecord, HubError> {
        let now = self.now();
        let thread = self.store.repository().thread(id)?;
        self.check_owns_thread(&thread, requester)?;
        let snapshot = self.store.repository().grouping_snapshot()?;
        let created_by = CreatedBy {
            agent: requester.agent.clone(),
            conversation_id: requester.conversation_id.clone(),
        };
        let (target, new_topic) = match &request.topic {
            TopicRef::Id(by_id) => {
                let topic = snapshot
                    .topics
                    .iter()
                    .find(|topic| topic.id == by_id.id)
                    .ok_or_else(|| {
                        HubError::Grouping(GroupingError::UnknownTopic(by_id.id.clone()))
                    })?;
                (topic.id.clone(), None)
            }
            TopicRef::Key(by_key) => match snapshot
                .topics
                .iter()
                .find(|topic| topic.key.as_deref() == Some(by_key.key.as_str()))
            {
                Some(topic) => (topic.id.clone(), None),
                None => {
                    let topic =
                        topic_from(inline_topic(by_key), new_group_id(now), &created_by, now);
                    (topic.id.clone(), Some(topic))
                }
            },
        };
        self.store.transaction(|repo| {
            if let Some(topic) = &new_topic {
                repo.insert_topic(topic)?;
            }
            repo.move_thread(id, &target, now)
        })?;
        if new_topic.is_some() {
            self.dispatcher.send(HubEvent::TopicChanged(target.clone()));
        }
        self.dispatcher.send(HubEvent::ThreadChanged(id.clone()));
        Ok(self.store.repository().thread_record(id)?)
    }

    /// Moving a thread moves its messages, so an agent may move only a thread
    /// it created and whose every message is its own.
    fn check_owns_thread(&self, thread: &Thread, requester: &Requester) -> Result<(), HubError> {
        if thread.created_by.agent != requester.agent {
            return Err(HubError::NotOwner);
        }
        let messages = self.store.repository().list_messages(&MessageFilter {
            states: Some(LifecycleState::ALL.to_vec()),
            thread_id: Some(thread.id.clone()),
            ..MessageFilter::default()
        })?;
        if messages.iter().all(|message| requester.owns(message)) {
            Ok(())
        } else {
            Err(HubError::NotOwner)
        }
    }

    pub(crate) fn mute_thread(
        &mut self,
        id: &ThreadId,
        is_muted: bool,
    ) -> Result<ThreadRecord, HubError> {
        let mut thread = self.store.repository().thread(id)?;
        thread.muted = is_muted;
        thread.updated_at = self.now();
        self.store.repository().update_thread(&thread)?;
        self.dispatcher.send(HubEvent::ThreadChanged(id.clone()));
        Ok(self.store.repository().thread_record(id)?)
    }

    pub(crate) fn mark_thread_read(&mut self, id: &ThreadId) -> Result<Vec<MessageId>, HubError> {
        let acknowledged = self.store.repository().mark_thread_read(id, self.now())?;
        self.after_mark_read(&acknowledged)?;
        self.dispatcher.send(HubEvent::ThreadChanged(id.clone()));
        Ok(acknowledged)
    }

    fn after_mark_read(&mut self, acknowledged: &[MessageId]) -> Result<(), HubError> {
        let now = self.now();
        for id in acknowledged {
            self.timers.cancel_repings(id);
            let message = self.store.repository().message(id)?;
            self.send_updated(&message, DeliveredSurfaces::default(), now);
        }
        Ok(())
    }

    // -----------------------------------------------------------------
    // Messages: move
    // -----------------------------------------------------------------

    pub(crate) fn move_message(
        &mut self,
        id: &MessageId,
        request: &MessageThreadMove,
        requester: &Requester,
    ) -> Result<MessageGrouping, HubError> {
        let now = self.now();
        let message = self.store.repository().message(id)?;
        if !requester.owns(&message) {
            return Err(HubError::NotOwner);
        }
        let topic = request.topic.as_ref().map(redacted_topic_ref);
        let thread = request.thread.as_ref().map(redacted_thread_ref);
        let snapshot = self.store.repository().grouping_snapshot()?;
        let resolution = resolve_grouping(
            &snapshot,
            &GroupingRequest {
                agent: &message.source.agent,
                conversation_id: &message.conversation.id,
                topic: topic.as_ref(),
                thread: thread.as_ref(),
                rule_topic: None,
                rule_thread: None,
                conversation_title: message.conversation.title.as_deref(),
                project: message.conversation.project_name(),
            },
        )
        .map_err(HubError::Grouping)?;
        let created_by = CreatedBy {
            agent: message.source.agent.clone(),
            conversation_id: Some(message.conversation.id.clone()),
        };
        let groups = PlannedGroups::new(&snapshot, resolution, &created_by, now);
        self.store.transaction(|repo| {
            groups.write(repo, &message, now)?;
            repo.move_message_to_thread(id, &groups.thread_id, now)
        })?;
        self.announce_groups(&groups);
        let moved = self.store.repository().message(id)?;
        self.send_updated(&moved, DeliveredSurfaces::default(), now);
        Ok(MessageGrouping {
            message_id: id.clone(),
            topic_id: groups.topic_id.clone(),
            thread_id: groups.thread_id.clone(),
        })
    }

    // -----------------------------------------------------------------
    // The UI: answers, acknowledge, snooze, dismiss, ticks
    // -----------------------------------------------------------------

    pub(crate) fn take_action(
        &mut self,
        id: &MessageId,
        request: &ActionRequest,
    ) -> Result<ActionOutcome, HubError> {
        let now = self.now();
        let message = self.store.repository().message(id)?;
        if message.lifecycle.state.is_closed() {
            return Err(HubError::MessageClosed {
                state: message.lifecycle.state,
                resolution: message.lifecycle.resolution,
            });
        }
        let action = message
            .actions
            .iter()
            .find(|action| action.id == request.action_id)
            .cloned()
            .ok_or(HubError::UnknownAction)?;
        let has_live_waiter = self.waiters.is_live(id, now);
        let is_offered = offered_actions(&message, has_live_waiter)
            .iter()
            .any(|offered| offered.id == action.id);
        if !is_offered {
            return Err(HubError::ActionNotOffered);
        }
        let press = ActionPress {
            kind: action.kind,
            action_id: Some(action.id.clone()),
            value: answer_value(&action, request.value.as_deref())?,
            via: Some(request.via),
            snooze_minutes: snooze_minutes_of(&action),
        };
        let outcome = press_action(&message, &press, now)?;
        self.apply_press(action.kind, &outcome, now)?;
        Ok(outcome)
    }

    pub(crate) fn acknowledge(&mut self, id: &MessageId) -> Result<Message, HubError> {
        let now = self.now();
        let message = self.store.repository().message(id)?;
        let outcome = press_action(&message, &ActionPress::of_kind(ActionKind::Open), now)?;
        self.apply_press(ActionKind::Open, &outcome, now)?;
        Ok(outcome.message)
    }

    pub(crate) fn snooze(
        &mut self,
        id: &MessageId,
        minutes: Option<u32>,
    ) -> Result<Message, HubError> {
        let now = self.now();
        let message = self.store.repository().message(id)?;
        let mut press = ActionPress::of_kind(ActionKind::Snooze);
        press.snooze_minutes = minutes;
        let outcome = press_action(&message, &press, now)?;
        self.apply_press(ActionKind::Snooze, &outcome, now)?;
        Ok(outcome.message)
    }

    /// Stores a press and carries out what it means for waiters, timers and
    /// surfaces.
    fn apply_press(
        &mut self,
        kind: ActionKind,
        outcome: &ActionOutcome,
        now: OffsetDateTime,
    ) -> Result<(), HubError> {
        let message = &outcome.message;
        self.store.repository().update_message(message, now)?;
        match kind {
            ActionKind::Approve
            | ActionKind::Deny
            | ActionKind::Choice
            | ActionKind::Reply
            | ActionKind::Done => {
                let take_down = self.after_close(message, now);
                self.dispatcher.send(HubEvent::Closed {
                    message_id: message.id.clone(),
                    state: message.lifecycle.state,
                    take_down,
                });
            }
            ActionKind::Defer => {
                // Defer is an answer that does not close: the waiter hears
                // `deferred` once, the popup closes, re-pings stop, and the
                // message is never delivered again as non_blocking.
                self.waiters
                    .defer(ActionResponse::for_message(message, true));
                self.timers.cancel_repings(&message.id);
                let take_down = self.take_down_popup(message);
                self.send_updated(message, take_down, now);
            }
            ActionKind::Open => {
                self.timers.cancel_repings(&message.id);
                self.send_updated(message, DeliveredSurfaces::default(), now);
            }
            ActionKind::Snooze => {
                self.timers.cancel_repings(&message.id);
                if let Some(until) = message.lifecycle.snoozed_until {
                    self.timers
                        .schedule(until, TimerKind::SnoozeEnd(message.id.clone()));
                }
                self.send_updated(message, DeliveredSurfaces::default(), now);
            }
        }
        Ok(())
    }

    /// After Defer the message is `non_blocking`: its plan is re-decided for
    /// the record, but only the popup comes down; nothing is shown again.
    fn take_down_popup(&mut self, message: &Message) -> DeliveredSurfaces {
        let plan = self.plan_for_stored(message);
        let Some(delivery) = self.deliveries.get_mut(&message.id) else {
            return DeliveredSurfaces::default();
        };
        let take_down = DeliveredSurfaces {
            popup: delivery.surfaces.popup,
            ..DeliveredSurfaces::default()
        };
        delivery.surfaces.popup = false;
        if let Ok(plan) = plan {
            delivery.plan = plan;
        }
        take_down
    }

    pub(crate) fn dismiss(&mut self, id: &MessageId) -> Result<Message, HubError> {
        let now = self.now();
        let message = self.store.repository().message(id)?;
        let dismissed = dismiss(&message, now)?;
        self.store.repository().update_message(&dismissed, now)?;
        let take_down = self.after_close(&dismissed, now);
        self.dispatcher.send(HubEvent::Closed {
            message_id: dismissed.id.clone(),
            state: dismissed.lifecycle.state,
            take_down,
        });
        Ok(dismissed)
    }

    pub(crate) fn tick_step(
        &mut self,
        id: &MessageId,
        index: usize,
        is_ticked: bool,
    ) -> Result<Message, HubError> {
        let now = self.now();
        let mut message = self.store.repository().message(id)?;
        if message.lifecycle.state.is_closed() {
            return Err(HubError::MessageClosed {
                state: message.lifecycle.state,
                resolution: message.lifecycle.resolution,
            });
        }
        message.steps_completed = set_step_ticked(
            &message.steps_completed,
            index,
            is_ticked,
            message.steps.len(),
        )
        .map_err(|_| HubError::UnknownStep(index))?;
        // Ticks are progress, never an answer: the waiter hears nothing.
        self.store.repository().update_message(&message, now)?;
        self.send_updated(&message, DeliveredSurfaces::default(), now);
        Ok(message)
    }

    pub(crate) fn offered_actions_of(&self, id: &MessageId) -> Result<Vec<Action>, HubError> {
        let message = self.store.repository().message(id)?;
        Ok(cloned(offered_actions(
            &message,
            self.waiters.is_live(id, self.now()),
        )))
    }

    fn send_updated(&self, message: &Message, take_down: DeliveredSurfaces, now: OffsetDateTime) {
        self.dispatcher.send(HubEvent::Updated {
            offered_actions: cloned(offered_actions(
                message,
                self.waiters.is_live(&message.id, now),
            )),
            message: message.clone(),
            take_down,
        });
    }

    // -----------------------------------------------------------------
    // Settings and presence
    // -----------------------------------------------------------------

    pub(crate) fn set_presence(&mut self, presence: Presence) {
        self.presence = presence;
    }

    pub(crate) fn set_focus(&mut self, is_focused: bool) {
        self.is_focused = is_focused;
    }

    pub(crate) fn set_quiet_hours(&mut self, is_active: bool) {
        self.is_quiet_hours = is_active;
    }

    pub(crate) fn set_rules(&mut self, rules: RuleSet) {
        self.rules = rules;
    }

    pub(crate) fn set_policy(&mut self, policy: Policy) {
        self.policy = policy;
    }

    pub(crate) fn set_sources(&mut self, sources: BTreeMap<String, IngestSettings>) {
        self.sources = sources;
    }

    pub(crate) fn sweep(&mut self) -> Result<SweepReport, HubError> {
        let report = self.store.repository().sweep(self.now(), &self.retention)?;
        tracing::info!(
            messages = report.messages,
            threads = report.threads,
            topics = report.topics,
            "retention sweep"
        );
        Ok(report)
    }

    // -----------------------------------------------------------------
    // Timers
    // -----------------------------------------------------------------

    pub(crate) fn fire_due_timers(&mut self) {
        let now = self.now();
        // Each timer on its own: one that fails or panics never costs the
        // others in the batch.
        for timer in self.timers.take_due(now) {
            guarded("a hub timer", || {
                let result = match &timer {
                    TimerKind::Reping(id) => self.on_reping(id, now),
                    TimerKind::SnoozeEnd(id) => self.on_snooze_end(id, now),
                    TimerKind::Expiry(id) => self.on_expiry(id, now),
                    TimerKind::WaiterLapse(id) => self.on_waiter_lapse(id, now),
                    TimerKind::Sweep => self.on_sweep(now),
                };
                if let Err(error) = result {
                    tracing::error!(?timer, %error, "a hub timer failed");
                }
            });
        }
    }

    /// Re-pings a message that is still `open`, with the plan decided now.
    fn on_reping(&mut self, id: &MessageId, now: OffsetDateTime) -> Result<(), HubError> {
        let Some(message) = self.store.repository().find_message(id)? else {
            return Ok(());
        };
        if message.lifecycle.state != LifecycleState::Open || !self.deliveries.contains_key(id) {
            return Ok(());
        }
        let plan = self.plan_for_stored(&message)?;
        let Some(reping) = plan.reping else {
            return Ok(());
        };
        let Some(delivery) = self.deliveries.get_mut(id) else {
            return Ok(());
        };
        if delivery.repings_sent >= reping.max {
            return Ok(());
        }
        delivery.repings_sent += 1;
        delivery.plan = plan.clone();
        if delivery.repings_sent < reping.max {
            self.timers
                .schedule(now + reping.after, TimerKind::Reping(id.clone()));
        }
        self.store.repository().append_delivery(&DeliveryRecord {
            message_id: id.clone(),
            recorded_at: now,
            event: DeliveryEvent::Repinged,
            surface: Some(surface_name(plan.mac).to_owned()),
            detail: None,
        })?;
        self.dispatcher.send(HubEvent::Repinged {
            message_id: id.clone(),
            plan,
        });
        Ok(())
    }

    /// A snooze ended: the message is `open` again with one re-ping, then the
    /// cell's cadence for what re-pings remain.
    fn on_snooze_end(&mut self, id: &MessageId, now: OffsetDateTime) -> Result<(), HubError> {
        let Some(message) = self.store.repository().find_message(id)? else {
            return Ok(());
        };
        let is_due = message.lifecycle.state == LifecycleState::Acknowledged
            && message
                .lifecycle
                .snoozed_until
                .is_some_and(|until| until <= now);
        if !is_due {
            return Ok(());
        }
        let mut reopened = message.clone();
        reopened.lifecycle = apply_event(&message.lifecycle, &LifecycleEvent::SnoozeEnded, now)
            .map_err(|_| HubError::MessageClosed {
                state: message.lifecycle.state,
                resolution: message.lifecycle.resolution,
            })?;
        self.store.repository().update_message(&reopened, now)?;
        let plan = self.plan_for_stored(&reopened)?;
        let repings_sent = self
            .deliveries
            .get(id)
            .map_or(0, |delivery| delivery.repings_sent);
        self.track_delivery(&reopened, &plan, repings_sent, now);
        self.send_updated(&reopened, DeliveredSurfaces::default(), now);
        self.dispatcher.send(HubEvent::Repinged {
            message_id: id.clone(),
            plan,
        });
        Ok(())
    }

    fn on_expiry(&mut self, id: &MessageId, now: OffsetDateTime) -> Result<(), HubError> {
        let Some(message) = self.store.repository().find_message(id)? else {
            return Ok(());
        };
        if !message.is_live() || message.expires_at > now {
            return Ok(());
        }
        let mut expired = message.clone();
        expired.lifecycle =
            apply_event(&message.lifecycle, &LifecycleEvent::Expire, now).map_err(|_| {
                HubError::MessageClosed {
                    state: message.lifecycle.state,
                    resolution: message.lifecycle.resolution,
                }
            })?;
        self.store.repository().update_message(&expired, now)?;
        let take_down = self.after_close(&expired, now);
        self.dispatcher.send(HubEvent::Closed {
            message_id: id.clone(),
            state: expired.lifecycle.state,
            take_down,
        });
        Ok(())
    }

    fn on_waiter_lapse(&mut self, id: &MessageId, now: OffsetDateTime) -> Result<(), HubError> {
        match self.store.repository().find_message(id)? {
            Some(message) if message.is_live() => self.announce_waiter(&message, now),
            _ => self.waiters.forget_if_idle(id, now),
        }
        Ok(())
    }

    fn on_sweep(&mut self, now: OffsetDateTime) -> Result<(), HubError> {
        self.timers.schedule(now + SWEEP_INTERVAL, TimerKind::Sweep);
        self.sweep().map(|_| ())
    }

    // -----------------------------------------------------------------
    // Helpers that need the state
    // -----------------------------------------------------------------

    /// The policy context now, with the message's mute.
    fn context(&self, is_muted: bool) -> PolicyContext {
        PolicyContext {
            presence: self.presence.clone(),
            focus_active: self.is_focused,
            quiet_hours_active: self.is_quiet_hours,
            is_muted,
            phone: None,
        }
    }

    /// The plan for a message; without an answer channel nobody can answer,
    /// so the sender should not wait.
    fn plan_for(&self, message: &Message, is_muted: bool) -> DeliveryPlan {
        let mut plan = decide(message, &self.context(is_muted), &self.policy);
        if !self.has_answer_channel {
            plan.should_wait = false;
            plan.offer_defer = false;
        }
        plan
    }

    /// The plan for a stored message, with the mute of its topic and thread.
    fn plan_for_stored(&self, message: &Message) -> Result<DeliveryPlan, HubError> {
        let repo = self.store.repository();
        let topic = repo.topic(&message.topic_id)?;
        let thread = repo.thread(&message.thread_id)?;
        Ok(self.plan_for(message, is_effectively_muted(&topic, &thread)))
    }

    fn current_should_wait(&self, message: &Message) -> Result<bool, HubError> {
        if !message.is_live() {
            return Ok(false);
        }
        Ok(self.plan_for_stored(message)?.should_wait)
    }

    fn item_of(&self, message: &Message, now: OffsetDateTime) -> OpenItem {
        let plan = self
            .deliveries
            .get(&message.id)
            .map(|delivery| delivery.plan.clone())
            .unwrap_or_else(|| self.plan_for(message, false));
        let mut item =
            OpenItem::from_message(message, self.waiters.is_live(&message.id, now), &plan);
        item.delivered = self
            .deliveries
            .get(&message.id)
            .map(|delivery| delivery.surfaces)
            .unwrap_or_default();
        item
    }

    fn open_items(
        &self,
        conversation: &ConversationKey,
        now: OffsetDateTime,
    ) -> Result<Vec<OpenItem>, HubError> {
        let messages = self.store.repository().list_messages(&MessageFilter {
            conversation: Some(conversation.clone()),
            ..MessageFilter::default()
        })?;
        Ok(messages
            .iter()
            .map(|message| self.item_of(message, now))
            .collect())
    }

    fn announce_groups(&self, groups: &PlannedGroups) {
        if groups.new_topic.is_some() || groups.reopens_topic.is_some() {
            self.dispatcher
                .send(HubEvent::TopicChanged(groups.topic_id.clone()));
        }
        if groups.new_thread.is_some() || groups.reopens_thread.is_some() {
            self.dispatcher
                .send(HubEvent::ThreadChanged(groups.thread_id.clone()));
        }
    }
}

/// A supersede: whose slot, and whether it pings.
#[derive(Debug, Clone)]
struct Supersede {
    replaces: MessageId,
    pings: bool,
}

/// The topic and thread a message lands in, with what must be created or
/// reopened first.
#[derive(Debug, Clone)]
struct PlannedGroups {
    topic_id: TopicId,
    thread_id: ThreadId,
    new_topic: Option<Topic>,
    new_thread: Option<Thread>,
    reopens_topic: Option<TopicId>,
    reopens_thread: Option<ThreadId>,
    uses_default_thread: bool,
    is_muted: bool,
    /// What the topic or thread mute is called, for the sender's warning.
    muted_group: Option<String>,
}

impl PlannedGroups {
    fn new(
        snapshot: &GroupingSnapshot,
        resolution: Resolution,
        created_by: &CreatedBy,
        now: OffsetDateTime,
    ) -> Self {
        let (topic_id, new_topic) = match resolution.topic {
            TopicTarget::Existing(id) => (id, None),
            TopicTarget::Create(new) => {
                let topic = topic_from(new, new_group_id(now), created_by, now);
                (topic.id.clone(), Some(topic))
            }
        };
        let (thread_id, new_thread) = match resolution.thread {
            ThreadTarget::Existing(id) => (id, None),
            ThreadTarget::Create(new) => {
                let thread = thread_from(new, new_group_id(now), &topic_id, created_by, now);
                (thread.id.clone(), Some(thread))
            }
        };
        let muted_topic = snapshot
            .topics
            .iter()
            .find(|topic| topic.id == topic_id && topic.muted);
        let muted_thread = snapshot
            .threads
            .iter()
            .find(|thread| thread.id == thread_id && thread.muted);
        let muted_group = muted_topic
            .map(|topic| format!("topic '{}'", topic.title))
            .or_else(|| muted_thread.map(|thread| format!("thread '{}'", thread.title)));
        Self {
            topic_id,
            thread_id,
            new_topic,
            new_thread,
            reopens_topic: resolution.reopens_topic,
            reopens_thread: resolution.reopens_thread,
            uses_default_thread: resolution.uses_default_thread,
            is_muted: muted_group.is_some(),
            muted_group,
        }
    }

    /// Creates and reopens what the message needs, and records the use of
    /// the conversation's default thread.
    fn write(
        &self,
        repo: &Repository<'_>,
        message: &Message,
        now: OffsetDateTime,
    ) -> Result<(), StoreError> {
        if let Some(topic) = &self.new_topic {
            repo.insert_topic(topic)?;
        }
        if let Some(thread) = &self.new_thread {
            repo.insert_thread(thread)?;
        }
        if let Some(thread) = &self.reopens_thread {
            repo.reopen_thread(thread, now)?;
        } else if let Some(topic) = &self.reopens_topic {
            repo.reopen_topic(topic, now)?;
        }
        if self.uses_default_thread {
            repo.touch_default_thread(
                &message.source.agent,
                &message.conversation.id,
                &self.topic_id,
                now,
            )?;
        }
        Ok(())
    }
}

/// Closes each withdrawn message and logs it. A message that closed in the
/// meantime is skipped.
fn close_withdrawn(
    repo: &Repository<'_>,
    rescissions: &[Rescission],
    now: OffsetDateTime,
) -> Result<Vec<(Message, Rescission)>, StoreError> {
    let mut closed = Vec::new();
    for rescission in rescissions {
        let mut message = repo.message(&rescission.message_id)?;
        let Ok(lifecycle) = apply_event(
            &message.lifecycle,
            &LifecycleEvent::Rescind(rescission.reason),
            now,
        ) else {
            continue;
        };
        message.lifecycle = lifecycle;
        repo.update_message(&message, now)?;
        repo.append_delivery(&DeliveryRecord {
            message_id: message.id.clone(),
            recorded_at: now,
            event: DeliveryEvent::Withdrawn,
            surface: None,
            detail: Some(rescission.reason.in_words().to_owned()),
        })?;
        closed.push((message, rescission.clone()));
    }
    Ok(closed)
}

/// The delivery log of a new message: where it went and why, and what the
/// hub changed.
fn log_delivery(
    repo: &Repository<'_>,
    message: &Message,
    plan: &DeliveryPlan,
    warnings: &[IngestWarning],
    now: OffsetDateTime,
) -> Result<(), StoreError> {
    let reasons = plan
        .reasons
        .iter()
        .map(|reason| format!("{reason:?}"))
        .collect::<Vec<_>>();
    repo.append_delivery(&DeliveryRecord {
        message_id: message.id.clone(),
        recorded_at: now,
        event: DeliveryEvent::Delivered,
        surface: Some(surface_name(plan.mac).to_owned()),
        detail: (!reasons.is_empty()).then(|| reasons.join(", ")),
    })?;
    for warning in warnings {
        let event = if warning.code == WarningCode::Normalized {
            DeliveryEvent::Normalized
        } else {
            DeliveryEvent::Warned
        };
        repo.append_delivery(&DeliveryRecord {
            message_id: message.id.clone(),
            recorded_at: now,
            event,
            surface: None,
            detail: Some(warning.message.clone()),
        })?;
    }
    Ok(())
}

fn rename_implicit_threads(
    repo: &Repository<'_>,
    key: &ConversationKey,
    title: &str,
    now: OffsetDateTime,
) -> Result<Vec<ThreadId>, StoreError> {
    let mut renamed = Vec::new();
    for link in repo.default_links()? {
        if link.agent != key.agent || link.conversation_id != key.conversation_id {
            continue;
        }
        let mut thread = repo.thread(&link.thread_id)?;
        if !thread.implicit || thread.title == title {
            continue;
        }
        thread.title = title.to_owned();
        thread.updated_at = now;
        repo.update_thread(&thread)?;
        renamed.push(thread.id);
    }
    Ok(renamed)
}

/// Tells the sender its message went to the inbox without a ping because a
/// mute holds it back (urgent messages break through and get no warning).
fn muted_warning(message: &Message, groups: &PlannedGroups) -> Option<IngestWarning> {
    if message.urgent {
        return None;
    }
    let cause = match (&groups.muted_group, &message.muted_by_rule) {
        (Some(group), _) => format!("{group} is muted"),
        (None, Some(rule)) => format!("rule '{rule}' mutes it"),
        (None, None) => return None,
    };
    Some(IngestWarning {
        code: WarningCode::Muted,
        field: None,
        message: format!("Delivered to the inbox without a ping: {cause}"),
    })
}

fn ingest_response(
    message: &Message,
    should_wait: bool,
    warnings: Vec<IngestWarning>,
) -> IngestResponse {
    IngestResponse {
        id: message.id.clone(),
        message_type: message.message_type,
        urgent: message.urgent,
        blocking: message.blocking,
        field_sources: Some(message.field_sources.clone()),
        topic_id: message.topic_id.clone(),
        thread_id: message.thread_id.clone(),
        state: message.lifecycle.state,
        should_wait,
        warnings,
    }
}

/// The value an answer carries: a choice returns its own value, a reply the
/// text you typed (required); the others none.
fn answer_value(action: &Action, typed: Option<&str>) -> Result<Option<String>, HubError> {
    match action.kind {
        ActionKind::Choice => Ok(action.value.clone()),
        ActionKind::Reply => {
            let text = typed
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .ok_or(HubError::InvalidAnswerValue("a reply needs text"))?;
            if text.chars().count() > MAX_REPLY_CHARS {
                return Err(HubError::InvalidAnswerValue(
                    "a reply is at most 8192 characters",
                ));
            }
            Ok(Some(text.to_owned()))
        }
        _ => Ok(None),
    }
}

fn snooze_minutes_of(action: &Action) -> Option<u32> {
    match action.kind {
        ActionKind::Snooze => action.value.as_deref().and_then(|value| value.parse().ok()),
        _ => None,
    }
}

/// A topic or thread title from an explicit PATCH: secrets redacted, and a
/// title that looks like raw input is refused (422).
fn guarded_group_title(title: &str) -> Result<String, HubError> {
    let redacted = redact_secrets(title).text;
    if !is_plain_words(&redacted) {
        return Err(HubError::Grouping(GroupingError::TitleLooksLikeCommand));
    }
    Ok(redacted)
}

fn redacted_topic_ref(reference: &TopicRef) -> TopicRef {
    match reference {
        TopicRef::Key(by_key) => {
            let mut by_key = by_key.clone();
            by_key.title = redact_optional(by_key.title.as_deref()).0;
            by_key.summary = redact_optional(by_key.summary.as_deref()).0;
            TopicRef::Key(by_key)
        }
        other => other.clone(),
    }
}

fn redacted_thread_ref(reference: &ThreadRef) -> ThreadRef {
    match reference {
        ThreadRef::Key(by_key) => {
            let mut by_key = by_key.clone();
            by_key.title = redact_optional(by_key.title.as_deref()).0;
            by_key.summary = redact_optional(by_key.summary.as_deref()).0;
            ThreadRef::Key(by_key)
        }
        other => other.clone(),
    }
}

fn inline_topic(by_key: &gentbox_core::topic::TopicKeyRef) -> NewTopic {
    let by_key = match redacted_topic_ref(&TopicRef::Key(by_key.clone())) {
        TopicRef::Key(by_key) => by_key,
        TopicRef::Id(_) => by_key.clone(),
    };
    NewTopic {
        key: Some(by_key.key.clone()),
        title: gentbox_core::title_guard::inline_group_title(&by_key.key, by_key.title.as_deref()),
        summary: by_key.summary.clone(),
        kind: by_key.kind.clone(),
        style: by_key.style.clone(),
        implicit: false,
        secrets_redacted: 0,
    }
}

fn topic_from(new: NewTopic, id: String, created_by: &CreatedBy, now: OffsetDateTime) -> Topic {
    Topic {
        id: TopicId::new(id),
        key: new.key,
        title: new.title,
        summary: new.summary,
        kind: new.kind,
        style: new.style,
        implicit: new.implicit,
        status: GroupStatus::Open,
        muted: false,
        marked_read_at: None,
        created_by: created_by.clone(),
        created_at: now,
        updated_at: now,
        closed_at: None,
    }
}

fn thread_from(
    new: NewThread,
    id: String,
    topic_id: &TopicId,
    created_by: &CreatedBy,
    now: OffsetDateTime,
) -> Thread {
    Thread {
        id: ThreadId::new(id),
        topic_id: topic_id.clone(),
        key: new.key,
        title: new.title,
        summary: new.summary,
        kind: new.kind,
        implicit: new.implicit,
        status: GroupStatus::Open,
        closed_by_topic: false,
        muted: false,
        marked_read_at: None,
        default_for: new.default_for,
        created_by: created_by.clone(),
        created_at: now,
        updated_at: now,
        closed_at: None,
    }
}

fn surface_name(surface: MacSurface) -> &'static str {
    match surface {
        MacSurface::PersistentPopup => "popup",
        MacSurface::Toast => "toast",
        MacSurface::Banner => "banner",
        MacSurface::InboxOnly => "inbox",
    }
}

fn cloned(actions: Vec<&Action>) -> Vec<Action> {
    actions.into_iter().cloned().collect()
}

fn ids_of(messages: &[Message]) -> Vec<MessageId> {
    messages.iter().map(|message| message.id.clone()).collect()
}

/// A new ULID for a message.
fn new_message_id(now: OffsetDateTime) -> MessageId {
    MessageId::new(new_ulid(now))
}

/// A new ULID for a topic or thread.
fn new_group_id(now: OffsetDateTime) -> String {
    new_ulid(now)
}

/// A ULID: the instant in milliseconds, then 80 random bits from the
/// operating system.
fn new_ulid(now: OffsetDateTime) -> String {
    let millis = u64::try_from(now.unix_timestamp_nanos() / 1_000_000).unwrap_or(0);
    let mut random = [0_u8; 16];
    // Without system randomness the hub cannot issue unguessable ids at
    // all; that is an environment defect, not an outcome to handle.
    #[allow(clippy::expect_used)]
    getrandom::fill(&mut random).expect("the operating system provides randomness");
    ulid::Ulid::from_parts(millis, u128::from_le_bytes(random)).to_string()
}
