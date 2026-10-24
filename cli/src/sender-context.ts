import { createHash, randomBytes } from 'node:crypto';
import type { SenderContext } from './build-message.ts';

export type SenderDefaults = {
	/** `source.agent` when `GENTBOX_AGENT` is unset. */
	agent: string;
	/** Conversation id when `GENTBOX_CONVERSATION` is unset. */
	conversationId: () => string;
};

const AGENT_PATTERN = /^[a-z0-9][a-z0-9-]{0,63}$/;
const TOKEN_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/;

/**
 * Who this process sends as: `GENTBOX_AGENT`, `GENTBOX_AGENT_NAME`,
 * `GENTBOX_CONVERSATION` and `GENTBOX_CONVERSATION_TITLE` (all self-asserted,
 * architecture §8), else the defaults. Throws on a malformed id, because a
 * wrong identity must stop the process rather than post under another name.
 */
export function readSenderContext(
	environment: NodeJS.ProcessEnv,
	defaults: SenderDefaults,
	workingDirectory: string = process.cwd(),
	now: () => Date = () => new Date(),
): SenderContext {
	const agent = environment.GENTBOX_AGENT || defaults.agent;
	if (!AGENT_PATTERN.test(agent)) {
		throw new Error(
			`GENTBOX_AGENT "${agent}" must be lowercase letters, digits and dashes (max 64)`,
		);
	}
	const conversationId =
		environment.GENTBOX_CONVERSATION || defaults.conversationId();
	if (!TOKEN_PATTERN.test(conversationId)) {
		throw new Error(
			`GENTBOX_CONVERSATION "${conversationId}" must be letters, digits and . _ : - (max 128)`,
		);
	}
	return {
		agent,
		displayName: environment.GENTBOX_AGENT_NAME || undefined,
		conversationId,
		conversationTitle: environment.GENTBOX_CONVERSATION_TITLE || undefined,
		workingDirectory,
		now,
	};
}

/** The same id for every CLI call from one folder, so its messages share a conversation. */
export function conversationIdForFolder(workingDirectory: string): string {
	return `cli-${createHash('sha256').update(workingDirectory).digest('hex').slice(0, 12)}`;
}

/** A fresh id for a process that lives as long as one session. */
export function newConversationId(prefix: string): string {
	return `${prefix}-${randomBytes(8).toString('hex')}`;
}
