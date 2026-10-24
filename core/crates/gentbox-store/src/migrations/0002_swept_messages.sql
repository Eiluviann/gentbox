-- Messages the retention sweep deleted, remembered so a long-poll for one can
-- answer `gone` instead of "unknown id" (architecture §5.2). Only the id and
-- when it was swept: a row is kept for the sweep's own retention period (a
-- year by default), then forgotten too.
CREATE TABLE swept_messages (
	id       TEXT PRIMARY KEY,
	swept_at INTEGER NOT NULL
) STRICT;

CREATE INDEX swept_messages_by_swept_at ON swept_messages (swept_at);
