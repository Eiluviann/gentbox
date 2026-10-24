-- Marks a rescinded message as cleared by the hub because it was handled in
-- the session (a completion or progress call), not withdrawn by its sender:
-- the domain reason `handled_in_session`. It is stored as the reason
-- `resolved_elsewhere` plus this flag, because the CHECK on `rescind_reason`
-- cannot be changed without rebuilding the table. Rows rescinded before this
-- migration keep their reason and read as withdrawn.
ALTER TABLE messages
	ADD COLUMN rescind_handled_in_session INTEGER NOT NULL DEFAULT 0
		CHECK (rescind_handled_in_session IN (0, 1));

-- When the hub parked the message: its sender was told to collect the answer
-- later (it arrived muted, or its sender stopped waiting after a timeout), so
-- the completion and progress clean-ups leave it open. NULL when not parked.
ALTER TABLE messages ADD COLUMN parked_at INTEGER;
