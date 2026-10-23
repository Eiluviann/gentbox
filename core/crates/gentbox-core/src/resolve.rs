//! Resolving a message's topic and thread (ADRs 0008, 0009), and the creates
//! and takeover of `POST /v1/topics` and `POST /v1/threads`.
//!
//! Pure: the hub passes a [`GroupingSnapshot`] of what exists and gets back
//! what to use and what to create; it applies the result itself.

use thiserror::Error;
use time::OffsetDateTime;

use crate::ids::{ThreadId, TopicId};
use crate::secret_scan::{redact_optional, redact_secrets};
use crate::template::GroupSpec;
use crate::title_guard::{inline_group_title, is_plain_words};
use crate::topic::{
    CONVERSATION_THREAD_KIND, DefaultFor, GENERAL_TOPIC_KIND, GENERAL_TOPIC_TITLE, GroupStatus,
    Thread, ThreadCreate, ThreadRef, Topic, TopicCreate, TopicRef, TopicStyle,
};

/// A conversation's default thread in one topic, and when it was last used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultThreadLink {
    /// The agent.
    pub agent: String,
    /// The conversation.
    pub conversation_id: String,
    /// The topic the thread is in.
    pub topic_id: TopicId,
    /// The default thread.
    pub thread_id: ThreadId,
    /// When a default message last landed in it; decides "latest".
    pub last_used: OffsetDateTime,
}

/// What exists, as the hub sees it. A conversation has at most one default
/// thread per topic.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GroupingSnapshot {
    /// Every topic.
    pub topics: Vec<Topic>,
    /// Every thread.
    pub threads: Vec<Thread>,
    /// Every conversation's default threads.
    pub default_links: Vec<DefaultThreadLink>,
}

impl GroupingSnapshot {
    fn topic(&self, id: &TopicId) -> Option<&Topic> {
        self.topics.iter().find(|topic| &topic.id == id)
    }

    fn topic_by_key(&self, key: &str) -> Option<&Topic> {
        self.topics
            .iter()
            .find(|topic| topic.key.as_deref() == Some(key))
    }

    fn general_topic_of(&self, agent: &str) -> Option<&Topic> {
        self.topics
            .iter()
            .find(|topic| topic.is_general_topic_of(agent))
    }

    fn thread(&self, id: &ThreadId) -> Option<&Thread> {
        self.threads.iter().find(|thread| &thread.id == id)
    }

    fn thread_by_key(&self, topic_id: &TopicId, key: &str) -> Option<&Thread> {
        self.threads
            .iter()
            .find(|thread| &thread.topic_id == topic_id && thread.key.as_deref() == Some(key))
    }

    fn default_link(
        &self,
        agent: &str,
        conversation_id: &str,
        topic_id: &TopicId,
    ) -> Option<&DefaultThreadLink> {
        self.default_links.iter().find(|link| {
            link.agent == agent
                && link.conversation_id == conversation_id
                && &link.topic_id == topic_id
        })
    }

    fn latest_default_link(
        &self,
        agent: &str,
        conversation_id: &str,
    ) -> Option<&DefaultThreadLink> {
        self.default_links
            .iter()
            .filter(|link| link.agent == agent && link.conversation_id == conversation_id)
            .max_by_key(|link| link.last_used)
    }
}

/// A topic to create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTopic {
    /// Stable key; none for the implicit General topic.
    pub key: Option<String>,
    /// Title, through the guard.
    pub title: String,
    /// Summary.
    pub summary: Option<String>,
    /// Kind hint.
    pub kind: Option<String>,
    /// Style hints.
    pub style: Option<TopicStyle>,
    /// True for an agent's General topic.
    pub implicit: bool,
    /// How many secrets the scan replaced in the title and summary of an
    /// explicit create (inline ones are scanned with the message).
    pub secrets_redacted: usize,
}

/// A thread to create, in the topic the resolution chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewThread {
    /// Stable key; none for an implicit default thread.
    pub key: Option<String>,
    /// Title, through the guard.
    pub title: String,
    /// Summary.
    pub summary: Option<String>,
    /// Kind hint.
    pub kind: Option<String>,
    /// True for a conversation's implicit default thread.
    pub implicit: bool,
    /// The conversation this thread becomes the default thread of.
    pub default_for: Option<DefaultFor>,
    /// How many secrets the scan replaced in the title and summary of an
    /// explicit create (inline ones are scanned with the message).
    pub secrets_redacted: usize,
}

/// The topic a message lands in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopicTarget {
    /// An existing topic.
    Existing(TopicId),
    /// A topic to create first.
    Create(NewTopic),
}

/// The thread a message lands in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadTarget {
    /// An existing thread.
    Existing(ThreadId),
    /// A thread to create in the resolved topic.
    Create(NewThread),
}

/// Everything resolution needs to know about the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupingRequest<'a> {
    /// `source.agent`.
    pub agent: &'a str,
    /// `conversation.id`.
    pub conversation_id: &'a str,
    /// The topic the message names.
    pub topic: Option<&'a TopicRef>,
    /// The thread the message names.
    pub thread: Option<&'a crate::topic::ThreadRef>,
    /// The topic a rule assigns (placeholders filled in).
    pub rule_topic: Option<&'a GroupSpec>,
    /// The thread a rule assigns (placeholders filled in).
    pub rule_thread: Option<&'a GroupSpec>,
    /// The conversation title after the guard, for an implicit thread.
    pub conversation_title: Option<&'a str>,
    /// The project name, the implicit thread's title when there is no
    /// conversation title.
    pub project: Option<&'a str>,
}

/// The outcome of resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// The topic to use or create.
    pub topic: TopicTarget,
    /// The thread to use or create.
    pub thread: ThreadTarget,
    /// A closed topic the message reopens ("a message to a closed topic or
    /// thread reopens it").
    pub reopens_topic: Option<TopicId>,
    /// A closed thread the message reopens.
    pub reopens_thread: Option<ThreadId>,
    /// The message lands in the conversation's default thread: the hub
    /// records the use (it decides which default thread is the latest).
    pub uses_default_thread: bool,
}

/// A failed resolution or create.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GroupingError {
    /// A named topic id does not exist.
    #[error("unknown topic {0}")]
    UnknownTopic(TopicId),
    /// A named thread id does not exist.
    #[error("unknown thread {0}")]
    UnknownThread(ThreadId),
    /// `thread.id` fixes the topic, and `topic` names a different one (422
    /// `topic_conflict`).
    #[error("topic_conflict: the thread belongs to another topic than the one named")]
    TopicConflict,
    /// An explicit topic or thread create with a flagged title (422
    /// `title_looks_like_command`).
    #[error("title_looks_like_command: the title looks like a command, path, URL or raw input")]
    TitleLooksLikeCommand,
}

/// Resolves a message's topic and thread.
///
/// - `thread: { id }` fixes both; a `topic` that names another topic is a
///   [`GroupingError::TopicConflict`].
/// - **Topic:** the named one → a rule-assigned one → the topic of the
///   conversation's latest default thread → the agent's General topic.
/// - **Thread, within that topic:** the named key (created on first use) → a
///   rule-assigned one → the conversation's default thread in that topic: an
///   implicit thread created on first use.
///
/// `kind` hints never enter the decision.
pub fn resolve_grouping(
    snapshot: &GroupingSnapshot,
    request: &GroupingRequest<'_>,
) -> Result<Resolution, GroupingError> {
    if let Some(ThreadRef::Id(thread_ref)) = request.thread {
        return resolve_by_thread_id(snapshot, request, &thread_ref.id);
    }
    let topic = resolve_topic(
        snapshot,
        request.agent,
        Some(request.conversation_id),
        request.topic,
        request.rule_topic,
    )?;
    let topic_id = existing_topic_id(&topic);
    let (thread, uses_default_thread) = resolve_thread(snapshot, request, topic_id)?;
    Ok(finish(snapshot, topic, thread, uses_default_thread))
}

fn resolve_by_thread_id(
    snapshot: &GroupingSnapshot,
    request: &GroupingRequest<'_>,
    thread_id: &ThreadId,
) -> Result<Resolution, GroupingError> {
    let thread = snapshot
        .thread(thread_id)
        .ok_or_else(|| GroupingError::UnknownThread(thread_id.clone()))?;
    let topic = snapshot
        .topic(&thread.topic_id)
        .ok_or_else(|| GroupingError::UnknownTopic(thread.topic_id.clone()))?;
    let conflicts = match request.topic {
        Some(TopicRef::Id(named)) => named.id != topic.id,
        Some(TopicRef::Key(named)) => topic.key.as_deref() != Some(named.key.as_str()),
        None => false,
    };
    if conflicts {
        return Err(GroupingError::TopicConflict);
    }
    Ok(finish(
        snapshot,
        TopicTarget::Existing(topic.id.clone()),
        ThreadTarget::Existing(thread.id.clone()),
        false,
    ))
}

fn resolve_topic(
    snapshot: &GroupingSnapshot,
    agent: &str,
    conversation_id: Option<&str>,
    named: Option<&TopicRef>,
    rule: Option<&GroupSpec>,
) -> Result<TopicTarget, GroupingError> {
    match named {
        Some(TopicRef::Id(by_id)) => {
            return snapshot
                .topic(&by_id.id)
                .map(|topic| TopicTarget::Existing(topic.id.clone()))
                .ok_or_else(|| GroupingError::UnknownTopic(by_id.id.clone()));
        }
        Some(TopicRef::Key(by_key)) => {
            return Ok(topic_by_key_or_new(
                snapshot,
                &by_key.key,
                NewTopic {
                    key: Some(by_key.key.clone()),
                    title: inline_group_title(&by_key.key, by_key.title.as_deref()),
                    summary: by_key.summary.clone(),
                    kind: by_key.kind.clone(),
                    style: by_key.style.clone(),
                    implicit: false,
                    secrets_redacted: 0,
                },
            ));
        }
        None => {}
    }
    if let Some(spec) = rule {
        return Ok(topic_by_key_or_new(
            snapshot,
            &spec.key,
            NewTopic {
                key: Some(spec.key.clone()),
                title: spec.title.clone(),
                summary: None,
                kind: spec.kind.clone(),
                style: None,
                implicit: false,
                secrets_redacted: 0,
            },
        ));
    }
    let latest_topic = conversation_id
        .and_then(|conversation_id| snapshot.latest_default_link(agent, conversation_id))
        .and_then(|link| snapshot.topic(&link.topic_id));
    if let Some(topic) = latest_topic {
        return Ok(TopicTarget::Existing(topic.id.clone()));
    }
    Ok(match snapshot.general_topic_of(agent) {
        Some(general) => TopicTarget::Existing(general.id.clone()),
        None => TopicTarget::Create(NewTopic {
            key: None,
            title: GENERAL_TOPIC_TITLE.to_owned(),
            summary: None,
            kind: Some(GENERAL_TOPIC_KIND.to_owned()),
            style: None,
            implicit: true,
            secrets_redacted: 0,
        }),
    })
}

fn topic_by_key_or_new(snapshot: &GroupingSnapshot, key: &str, new: NewTopic) -> TopicTarget {
    match snapshot.topic_by_key(key) {
        Some(existing) => TopicTarget::Existing(existing.id.clone()),
        None => TopicTarget::Create(new),
    }
}

fn existing_topic_id(target: &TopicTarget) -> Option<&TopicId> {
    match target {
        TopicTarget::Existing(id) => Some(id),
        TopicTarget::Create(_) => None,
    }
}

fn resolve_thread(
    snapshot: &GroupingSnapshot,
    request: &GroupingRequest<'_>,
    topic_id: Option<&TopicId>,
) -> Result<(ThreadTarget, bool), GroupingError> {
    if let Some(ThreadRef::Key(named)) = request.thread {
        let target = thread_by_key_or_new(
            snapshot,
            topic_id,
            &named.key,
            NewThread {
                key: Some(named.key.clone()),
                title: inline_group_title(&named.key, named.title.as_deref()),
                summary: named.summary.clone(),
                kind: named.kind.clone(),
                implicit: false,
                secrets_redacted: 0,
                default_for: None,
            },
        );
        return Ok((target, false));
    }
    if let Some(spec) = request.rule_thread {
        let target = thread_by_key_or_new(
            snapshot,
            topic_id,
            &spec.key,
            NewThread {
                key: Some(spec.key.clone()),
                title: spec.title.clone(),
                summary: None,
                kind: spec.kind.clone(),
                implicit: false,
                secrets_redacted: 0,
                default_for: None,
            },
        );
        return Ok((target, false));
    }
    let existing_default = topic_id.and_then(|topic_id| {
        snapshot.default_link(request.agent, request.conversation_id, topic_id)
    });
    let target = match existing_default {
        Some(link) => ThreadTarget::Existing(link.thread_id.clone()),
        None => ThreadTarget::Create(implicit_default_thread(request)),
    };
    Ok((target, true))
}

fn thread_by_key_or_new(
    snapshot: &GroupingSnapshot,
    topic_id: Option<&TopicId>,
    key: &str,
    new: NewThread,
) -> ThreadTarget {
    let existing = topic_id.and_then(|topic_id| snapshot.thread_by_key(topic_id, key));
    match existing {
        Some(thread) => ThreadTarget::Existing(thread.id.clone()),
        None => ThreadTarget::Create(new),
    }
}

fn implicit_default_thread(request: &GroupingRequest<'_>) -> NewThread {
    let title = request
        .conversation_title
        .filter(|title| is_plain_words(title))
        .or(request.project.filter(|project| is_plain_words(project)))
        .map(str::to_owned)
        .unwrap_or_else(|| "Conversation".to_owned());
    NewThread {
        key: None,
        title,
        summary: None,
        kind: Some(CONVERSATION_THREAD_KIND.to_owned()),
        implicit: true,
        secrets_redacted: 0,
        default_for: Some(DefaultFor {
            agent: request.agent.to_owned(),
            conversation_id: request.conversation_id.to_owned(),
        }),
    }
}

fn finish(
    snapshot: &GroupingSnapshot,
    topic: TopicTarget,
    thread: ThreadTarget,
    uses_default_thread: bool,
) -> Resolution {
    let reopens_thread = match &thread {
        ThreadTarget::Existing(id) => snapshot
            .thread(id)
            .filter(|thread| thread.status == GroupStatus::Closed)
            .map(|thread| thread.id.clone()),
        ThreadTarget::Create(_) => None,
    };
    let reopens_topic = match &topic {
        TopicTarget::Existing(id) => snapshot
            .topic(id)
            .filter(|topic| topic.status == GroupStatus::Closed)
            .map(|topic| topic.id.clone()),
        TopicTarget::Create(_) => None,
    };
    Resolution {
        topic,
        thread,
        reopens_topic,
        reopens_thread,
        uses_default_thread,
    }
}

/// The outcome of `POST /v1/topics`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopicCreateOutcome {
    /// The key exists: 200, returned unchanged.
    Existing(TopicId),
    /// A new topic: 201.
    Created(NewTopic),
}

/// `POST /v1/topics`: an upsert by key. The title must pass the guard (an
/// explicit create is not time-critical, so the agent fixes it).
pub fn create_topic(
    snapshot: &GroupingSnapshot,
    request: &TopicCreate,
) -> Result<TopicCreateOutcome, GroupingError> {
    if let Some(existing) = snapshot.topic_by_key(&request.key) {
        return Ok(TopicCreateOutcome::Existing(existing.id.clone()));
    }
    let title = redact_secrets(&request.title);
    let (summary, summary_redactions) = redact_optional(request.summary.as_deref());
    if !is_plain_words(&title.text) {
        return Err(GroupingError::TitleLooksLikeCommand);
    }
    Ok(TopicCreateOutcome::Created(NewTopic {
        key: Some(request.key.clone()),
        title: title.text,
        summary,
        kind: request.kind.clone(),
        style: request.style.clone(),
        implicit: false,
        secrets_redacted: title.redactions + summary_redactions,
    }))
}

/// A conversation's implicit thread that an explicit one replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Takeover {
    /// The implicit thread to delete after moving its messages in.
    pub replaces: ThreadId,
    /// The new thread is muted when the implicit one was.
    pub inherits_mute: bool,
}

/// What `POST /v1/threads` does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadCreateAction {
    /// The key exists in the topic: 200, returned unchanged (no takeover).
    Existing(ThreadId),
    /// A new thread: 201, with the takeover it performs, if any.
    Created {
        /// The thread to create; `default_for` says whether it becomes the
        /// conversation's default thread.
        thread: NewThread,
        /// The implicit thread it takes over.
        takeover: Option<Takeover>,
    },
}

/// The outcome of `POST /v1/threads`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadCreateOutcome {
    /// The topic the thread lives in, to create first when new.
    pub topic: TopicTarget,
    /// What happens to the thread.
    pub action: ThreadCreateAction,
}

/// Context of a thread create that the request body does not carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadCreateContext<'a> {
    /// The creating agent.
    pub agent: &'a str,
    /// The topic a rule assigns when the request names none.
    pub rule_topic: Option<&'a GroupSpec>,
}

/// `POST /v1/threads`: an upsert by key within the topic.
///
/// Without `topic` the topic is resolved as for a message. An explicit thread
/// created with `kind: "conversation"` and `conversation.id` becomes that
/// conversation's default thread in the topic and **takes over** its implicit
/// thread, if it has one: the first one only, and never an existing key (the
/// create is idempotent).
pub fn create_thread(
    snapshot: &GroupingSnapshot,
    request: &ThreadCreate,
    context: &ThreadCreateContext<'_>,
) -> Result<ThreadCreateOutcome, GroupingError> {
    let conversation_id = request
        .conversation
        .as_ref()
        .map(|conversation| conversation.id.as_str());
    let topic = resolve_topic(
        snapshot,
        context.agent,
        conversation_id,
        request.topic.as_ref(),
        context.rule_topic,
    )?;
    let topic_id = existing_topic_id(&topic);
    if let Some(existing) = topic_id.and_then(|id| snapshot.thread_by_key(id, &request.key)) {
        return Ok(ThreadCreateOutcome {
            topic,
            action: ThreadCreateAction::Existing(existing.id.clone()),
        });
    }
    let title = redact_secrets(&request.title);
    let (summary, summary_redactions) = redact_optional(request.summary.as_deref());
    if !is_plain_words(&title.text) {
        return Err(GroupingError::TitleLooksLikeCommand);
    }
    let wants_default = request.kind.as_deref() == Some(CONVERSATION_THREAD_KIND);
    let (default_for, takeover) = match (wants_default, conversation_id) {
        (true, Some(conversation_id)) => {
            default_and_takeover(snapshot, context.agent, conversation_id, topic_id)
        }
        _ => (None, None),
    };
    let thread = NewThread {
        key: Some(request.key.clone()),
        title: title.text,
        summary,
        kind: request.kind.clone(),
        implicit: false,
        secrets_redacted: title.redactions + summary_redactions,
        default_for,
    };
    Ok(ThreadCreateOutcome {
        topic,
        action: ThreadCreateAction::Created { thread, takeover },
    })
}

fn default_and_takeover(
    snapshot: &GroupingSnapshot,
    agent: &str,
    conversation_id: &str,
    topic_id: Option<&TopicId>,
) -> (Option<DefaultFor>, Option<Takeover>) {
    let default_for = DefaultFor {
        agent: agent.to_owned(),
        conversation_id: conversation_id.to_owned(),
    };
    let current_default = topic_id
        .and_then(|topic_id| snapshot.default_link(agent, conversation_id, topic_id))
        .and_then(|link| snapshot.thread(&link.thread_id));
    match current_default {
        None => (Some(default_for), None),
        Some(thread) if thread.implicit => (
            Some(default_for),
            Some(Takeover {
                replaces: thread.id.clone(),
                inherits_mute: thread.muted,
            }),
        ),
        // The default thread is already explicit: an ordinary thread.
        Some(_) => (None, None),
    }
}
