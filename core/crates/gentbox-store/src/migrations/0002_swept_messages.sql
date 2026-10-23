-- Messages the retention sweep deleted, remembered so a long-poll for one can
-- answer `gone` instead of "unknown id" (architecture §5.2). A row is kept for
-- the sweep's own retention period, then forgotten too.
CREATE TABLE swept_messages (
	id           TEXT PRIMARY KEY,
	source_agent TEXT NOT NULL,
	closed_at    INTEGER NOT NULL,
	swept_at     INTEGER NOT NULL
) STRICT;

CREATE INDEX swept_messages_by_swept_at ON swept_messages (swept_at);
