-- Gentbox store, first schema (PR 2a).
--
-- Timestamps are INTEGER nanoseconds since the Unix epoch (UTC), so they sort
-- and compare in SQL and round-trip exactly. Booleans are INTEGER 0/1. Enums
-- are the snake_case names core uses on the wire. Nested records that are only
-- ever read and written whole (steps, metadata, field sources, focus target,
-- style) are JSON text.
--
-- Topics and threads: docs/adr/0008-topics.md, docs/adr/0009-threads.md.

CREATE TABLE topics (
	id                         TEXT PRIMARY KEY,
	key                        TEXT UNIQUE,
	title                      TEXT NOT NULL,
	summary                    TEXT,
	kind                       TEXT,
	style                      TEXT,
	implicit                   INTEGER NOT NULL CHECK (implicit IN (0, 1)),
	status                     TEXT NOT NULL CHECK (status IN ('open', 'closed')),
	muted                      INTEGER NOT NULL DEFAULT 0 CHECK (muted IN (0, 1)),
	marked_read_at             INTEGER,
	created_by_agent           TEXT NOT NULL,
	created_by_conversation_id TEXT,
	created_at                 INTEGER NOT NULL,
	updated_at                 INTEGER NOT NULL,
	closed_at                  INTEGER,
	CHECK (implicit = 0 OR key IS NULL),
	CHECK ((closed_at IS NOT NULL) = (status = 'closed'))
) STRICT;

-- At most one implicit General topic per agent.
CREATE UNIQUE INDEX topics_one_general_per_agent
	ON topics (created_by_agent)
	WHERE implicit = 1 AND kind = 'general';

CREATE TABLE threads (
	id                         TEXT PRIMARY KEY,
	topic_id                   TEXT NOT NULL REFERENCES topics (id),
	key                        TEXT,
	title                      TEXT NOT NULL,
	summary                    TEXT,
	kind                       TEXT,
	implicit                   INTEGER NOT NULL CHECK (implicit IN (0, 1)),
	status                     TEXT NOT NULL CHECK (status IN ('open', 'closed')),
	closed_by_topic            INTEGER NOT NULL DEFAULT 0 CHECK (closed_by_topic IN (0, 1)),
	muted                      INTEGER NOT NULL DEFAULT 0 CHECK (muted IN (0, 1)),
	marked_read_at             INTEGER,
	created_by_agent           TEXT NOT NULL,
	created_by_conversation_id TEXT,
	created_at                 INTEGER NOT NULL,
	updated_at                 INTEGER NOT NULL,
	closed_at                  INTEGER,
	-- A key is unique within its topic; threads without a key never collide.
	UNIQUE (topic_id, key),
	-- The target of messages' composite foreign key.
	UNIQUE (id, topic_id),
	CHECK (implicit = 0 OR key IS NULL),
	CHECK ((closed_at IS NOT NULL) = (status = 'closed')),
	CHECK (closed_by_topic = 0 OR status = 'closed')
) STRICT;

CREATE INDEX threads_by_topic ON threads (topic_id, status);

CREATE TABLE conversations (
	agent           TEXT NOT NULL,
	conversation_id TEXT NOT NULL,
	title           TEXT,
	cwd             TEXT,
	focus           TEXT,
	updated_at      INTEGER NOT NULL,
	PRIMARY KEY (agent, conversation_id)
) STRICT;

-- A conversation's default thread in a topic: at most one per topic, and a
-- thread is the default of at most one conversation.
CREATE TABLE conversation_default_threads (
	agent           TEXT NOT NULL,
	conversation_id TEXT NOT NULL,
	topic_id        TEXT NOT NULL REFERENCES topics (id) ON DELETE CASCADE,
	thread_id       TEXT NOT NULL UNIQUE REFERENCES threads (id) ON DELETE CASCADE,
	last_used       INTEGER NOT NULL,
	PRIMARY KEY (agent, conversation_id, topic_id),
	-- The link's topic is always the thread's topic; a moved thread carries
	-- its link along.
	FOREIGN KEY (thread_id, topic_id)
		REFERENCES threads (id, topic_id) ON UPDATE CASCADE ON DELETE CASCADE
) STRICT;

CREATE TABLE messages (
	id                    TEXT PRIMARY KEY,
	idempotency_key       TEXT,
	source_agent          TEXT NOT NULL,
	source_display_name   TEXT,
	source_version        TEXT,
	source_host           TEXT,
	conversation_id       TEXT NOT NULL,
	topic_id              TEXT NOT NULL,
	thread_id             TEXT NOT NULL,
	type                  TEXT NOT NULL
		CHECK (type IN ('decision_required', 'action_required', 'task_completed', 'generic')),
	urgent                INTEGER NOT NULL CHECK (urgent IN (0, 1)),
	blocking              TEXT NOT NULL CHECK (blocking IN ('non_blocking', 'deferrable', 'blocking')),
	field_sources         TEXT NOT NULL,
	muted_by_rule         TEXT,
	event                 TEXT,
	title                 TEXT NOT NULL,
	summary               TEXT,
	body                  TEXT,
	steps                 TEXT NOT NULL DEFAULT '[]',
	steps_completed       TEXT NOT NULL DEFAULT '[]',
	scope                 TEXT CHECK (scope IN ('task', 'goal')),
	awaits_response       INTEGER NOT NULL CHECK (awaits_response IN (0, 1)),
	correlation_key       TEXT,
	sensitive             INTEGER NOT NULL CHECK (sensitive IN (0, 1)),
	created_at            INTEGER,
	received_at           INTEGER NOT NULL,
	expires_at            INTEGER NOT NULL,
	metadata              TEXT NOT NULL DEFAULT '{}',
	repeat_count          INTEGER NOT NULL DEFAULT 0 CHECK (repeat_count >= 0),
	state                 TEXT NOT NULL
		CHECK (state IN ('open', 'acknowledged', 'answered', 'dismissed', 'rescinded', 'expired')),
	snoozed_until         INTEGER,
	deferred_at           INTEGER,
	resolution            TEXT CHECK (resolution IN ('answered', 'dismissed', 'rescinded', 'expired')),
	rescind_reason        TEXT
		CHECK (rescind_reason IN ('resolved_elsewhere', 'obsolete', 'superseded')),
	-- The message that replaced this one: the supersede chain.
	superseded_by         TEXT REFERENCES messages (id) ON DELETE SET NULL,
	-- When the message reached a terminal state; retention counts from here.
	closed_at             INTEGER,
	updated_at            INTEGER NOT NULL,
	FOREIGN KEY (source_agent, conversation_id)
		REFERENCES conversations (agent, conversation_id),
	-- The topic is denormalised from the thread and kept in step with it: a
	-- thread that moves to another topic carries its messages along.
	FOREIGN KEY (thread_id, topic_id)
		REFERENCES threads (id, topic_id) ON UPDATE CASCADE,
	CHECK (rescind_reason IS NULL OR resolution = 'rescinded'),
	CHECK (resolution IS NOT 'rescinded' OR rescind_reason IS NOT NULL),
	-- A live message has no resolution; a closed one has the matching one.
	CHECK (CASE state
		WHEN 'answered' THEN resolution IS 'answered'
		WHEN 'dismissed' THEN resolution IS 'dismissed'
		WHEN 'rescinded' THEN resolution IS 'rescinded'
		WHEN 'expired' THEN resolution IS 'expired'
		ELSE resolution IS NULL
	END),
	CHECK ((closed_at IS NOT NULL) = (state IN ('answered', 'dismissed', 'rescinded', 'expired')))
) STRICT;

CREATE UNIQUE INDEX messages_idempotency
	ON messages (source_agent, idempotency_key)
	WHERE idempotency_key IS NOT NULL;
CREATE INDEX messages_by_state ON messages (state, received_at);
CREATE INDEX messages_by_conversation ON messages (source_agent, conversation_id, state);
CREATE INDEX messages_by_topic ON messages (topic_id, state);
CREATE INDEX messages_by_thread ON messages (thread_id, state);
CREATE INDEX messages_by_expiry ON messages (expires_at) WHERE state IN ('open', 'acknowledged');
CREATE INDEX messages_by_snooze ON messages (snoozed_until) WHERE snoozed_until IS NOT NULL;
CREATE INDEX messages_by_closing ON messages (closed_at) WHERE closed_at IS NOT NULL;
CREATE INDEX messages_by_successor ON messages (superseded_by) WHERE superseded_by IS NOT NULL;

CREATE TABLE actions (
	message_id  TEXT NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
	position    INTEGER NOT NULL,
	action_id   TEXT NOT NULL,
	label       TEXT NOT NULL,
	kind        TEXT NOT NULL
		CHECK (kind IN ('approve', 'deny', 'choice', 'reply', 'done', 'defer', 'open', 'snooze')),
	value       TEXT,
	placeholder TEXT,
	destructive INTEGER NOT NULL DEFAULT 0 CHECK (destructive IN (0, 1)),
	PRIMARY KEY (message_id, position),
	UNIQUE (message_id, action_id)
) STRICT;

-- The answer that closed a message, kept so a sender that polls later (or
-- never waited) still hears it.
CREATE TABLE responses (
	message_id   TEXT PRIMARY KEY REFERENCES messages (id) ON DELETE CASCADE,
	action_id    TEXT NOT NULL,
	action_kind  TEXT NOT NULL
		CHECK (action_kind IN ('approve', 'deny', 'choice', 'reply', 'done', 'defer', 'open', 'snooze')),
	value        TEXT,
	answered_via TEXT CHECK (answered_via IN ('mac', 'phone', 'watch', 'cli')),
	answered_at  INTEGER NOT NULL
) STRICT;

-- The delivery log: what the hub did with a message and why.
CREATE TABLE deliveries (
	sequence    INTEGER PRIMARY KEY AUTOINCREMENT,
	message_id  TEXT NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
	recorded_at INTEGER NOT NULL,
	event       TEXT NOT NULL
		CHECK (event IN ('delivered', 'repinged', 'normalized', 'warned', 'withdrawn')),
	surface     TEXT,
	detail      TEXT
) STRICT;

CREATE INDEX deliveries_by_message ON deliveries (message_id, sequence);
