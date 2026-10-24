import { type ChildProcess, spawn } from 'node:child_process';
import {
	chmodSync,
	cpSync,
	existsSync,
	mkdtempSync,
	rmSync,
	writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { askAndWait } from './ask-flow.ts';
import { buildMessage, type MessageDraft } from './build-message.ts';
import { readDiscoveryEnvironment } from './discovery.ts';
import { connectToHub, type HubClient } from './hub-client.ts';
import { runCli } from './main.ts';
import { runTool } from './mcp-tool-handlers.ts';
import { succeed } from './result.ts';
import { TEST_SENDER } from './test-support/command-fixture.ts';
import { environmentOf } from './test-support/environment-of.ts';

// Runs against a real gentboxd built from core/ (`cd core && cargo build -p gentboxd`, or
// GENTBOXD_BIN); skipped when the binary is absent.
const REPOSITORY_ROOT = join(dirname(fileURLToPath(import.meta.url)), '../..');
const GENTBOXD =
	process.env.GENTBOXD_BIN ??
	join(REPOSITORY_ROOT, 'core/target/debug/gentboxd');
const isDaemonAvailable = existsSync(GENTBOXD);

const IDENTITY = { agent: 'claude-code', conversationId: 'integration-1' };

describe.skipIf(!isDaemonAvailable)('against a real gentboxd', () => {
	let stateDirectory: string;
	let daemon: ChildProcess;
	let apiDirectory: string;

	beforeAll(async () => {
		stateDirectory = mkdtempSync(join(tmpdir(), 'gentbox-it-'));
		writeFileSync(
			join(stateDirectory, 'rules.toml'),
			'[[rule]]\nname = "Mute integration asks"\nmatch = { event = "integration.muted" }\nmute = true\n',
		);
		apiDirectory = join(stateDirectory, 'API');
		daemon = spawn(
			GENTBOXD,
			['--state-dir', stateDirectory, '--config-dir', stateDirectory],
			{
				env: { ...process.env, ...environmentOf(['GENTBOX_LOG', 'warn']) },
				stdio: 'ignore',
			},
		);
		await waitUntil(() => existsSync(join(apiDirectory, 'port')));
	}, 30_000);

	afterAll(async () => {
		daemon.kill('SIGTERM');
		await new Promise((resolve) => daemon.once('exit', resolve));
		rmSync(stateDirectory, { recursive: true, force: true });
	});

	// One connection serves the tests that do not care about the transport: every
	// `hello` draws on the hub's shared, rate-limited bucket.
	let sharedClient: HubClient | undefined;

	async function connect(): Promise<HubClient> {
		if (sharedClient !== undefined) return sharedClient;
		const connected = await connectToHub({
			environment: readDiscoveryEnvironment(
				environmentOf(['GENTBOX_API_DIR', apiDirectory]),
			),
			identity: IDENTITY,
		});
		if (!connected.ok) throw new Error(connected.error);
		sharedClient = connected.value;
		return connected.value;
	}

	function message(title: string, overrides: Partial<MessageDraft> = {}) {
		const built = buildMessage(
			{ title, ...overrides },
			{ ...TEST_SENDER, conversationId: IDENTITY.conversationId },
		);
		if (!built.ok) throw new Error(built.error.join('; '));
		return built.value;
	}

	it('sends over the socket and gets the declared fields back', async () => {
		const client = await connect();
		const posted = await client.postMessage(
			message('Ingest auth accepts rotated tokens; 12 tests pass', {
				type: 'task_completed',
			}),
		);
		expect(posted).toMatchObject({
			ok: true,
			value: {
				wasCreated: true,
				record: {
					type: 'task_completed',
					blocking: 'non_blocking',
					state: 'open',
				},
			},
		});
	});

	it('passes the title guard warning through when a title looks like a command', async () => {
		const client = await connect();
		const posted = await client.postMessage(
			message('cargo test -p gentbox-ingest -- --nocapture', {
				type: 'generic',
			}),
		);
		expect(
			posted.ok && posted.value.record.warnings?.map((warning) => warning.code),
		).toContain('title_looks_like_command');
	});

	it('proves the hub over TCP when the socket is gone: the Rust and TypeScript hello proofs agree', async () => {
		rmSync(join(apiDirectory, 'hub.sock'));
		sharedClient = undefined;
		const client = await connect();
		const posted = await client.postMessage(
			message('Release notes drafted', { type: 'generic' }),
		);
		expect(posted.ok).toBe(true);
	});

	it('refuses a hub that does not hold the token: a different token never gets sent', async () => {
		// A second API folder with the real port but another token: the real hub cannot sign for it.
		const impostorDirectory = mkdtempSync(
			join(tmpdir(), 'gentbox-it-impostor-'),
		);
		try {
			cpSync(apiDirectory, join(impostorDirectory, 'API'), { recursive: true });
			writeFileSync(
				join(impostorDirectory, 'API/token'),
				'b3RoZXItdG9rZW4tdGhhdC10aGUtaHViLWRvZXMtbm90LWtub3c',
				{ mode: 0o600 },
			);
			chmodSync(join(impostorDirectory, 'API'), 0o700);
			const connected = await connectToHub({
				environment: readDiscoveryEnvironment(
					environmentOf(['GENTBOX_API_DIR', join(impostorDirectory, 'API')]),
				),
				identity: IDENTITY,
			});
			expect(connected.ok).toBe(false);
			if (!connected.ok)
				expect(connected.error).toContain('token was not sent');
		} finally {
			rmSync(impostorDirectory, { recursive: true, force: true });
		}
	});

	it('creates, lists and closes topics and threads', async () => {
		const client = await connect();
		const topic = await client.createTopic({
			key: 'release-1.4',
			title: 'Release 1.4',
			conversation: { id: IDENTITY.conversationId },
		});
		expect(topic).toMatchObject({
			ok: true,
			value: {
				wasCreated: true,
				record: { key: 'release-1.4', status: 'open' },
			},
		});
		if (!topic.ok) return;
		const thread = await client.createThread({
			topic: { id: topic.value.record.id },
			key: 'phase-3',
			title: 'Phase 3',
			conversation: { id: IDENTITY.conversationId },
		});
		expect(thread).toMatchObject({ ok: true, value: { wasCreated: true } });
		const listed = await client.listThreads(topic.value.record.id, 'open');
		expect(listed.ok && listed.value.map((record) => record.key)).toContain(
			'phase-3',
		);
		const closed = await client.closeTopic(topic.value.record.id, {});
		expect(closed).toMatchObject({ ok: true, value: { status: 'closed' } });
	});

	it('ask returns "no answer" at once and leaves the message open when no UI can answer', async () => {
		const client = await connect();
		const asked = await askAndWait(
			client,
			message('Allow Bash to run the ingest tests?', {
				type: 'decision_required',
				blocking: 'blocking',
			}),
			{ timeoutSeconds: 5 },
		);
		expect(asked).toMatchObject({
			ok: true,
			value: { outcome: { kind: 'not_waiting' } },
		});
		if (!asked.ok) return;
		const polled = await client.getResponse(asked.value.ingest.id, 0);
		expect(polled).toMatchObject({ ok: true, value: { status: 'pending' } });
		// Clean up: the poll above gives the item a live waiter for a while, and
		// the next test's asks would fold into it.
		await client.rescindMessage(asked.value.ingest.id, 'resolved_elsewhere');
	});

	it('a muted ask returns muted at once, names the mute, and stays open until the model rescinds it', async () => {
		const client = await connect();
		const asked = await askAndWait(
			client,
			message('Allow Bash to run the ingest tests?', {
				type: 'decision_required',
				blocking: 'blocking',
				event: 'integration.muted',
			}),
			{ timeoutSeconds: 5 },
		);
		expect(asked).toMatchObject({
			ok: true,
			value: { outcome: { kind: 'muted' } },
		});
		if (!asked.ok) return;
		expect(asked.value.ingest.warnings).toContainEqual({
			code: 'muted',
			message:
				"Delivered to the inbox without a ping: rule 'Mute integration asks' mutes it",
		});
		const stillOpen = await client.getResponse(asked.value.ingest.id, 0);
		expect(stillOpen).toMatchObject({ ok: true, value: { status: 'pending' } });
		// Only an explicit rescind from the agent withdraws it.
		const rescinded = await client.rescindMessage(
			asked.value.ingest.id,
			'resolved_elsewhere',
		);
		expect(rescinded).toMatchObject({
			ok: true,
			value: { status: 'rescinded', rescindReason: 'resolved_elsewhere' },
		});
	});

	it('a completion clears an earlier ask as handled in the session, an explicit rescind keeps its reason', async () => {
		const client = await connect();
		const stale = await client.postMessage(
			message('Pick the next phase', {
				type: 'decision_required',
				blocking: 'deferrable',
				correlationKey: 'handled-1',
			}),
		);
		if (!stale.ok) throw new Error('post failed');
		const done = await client.postMessage(
			message('Phase 3 is done', { type: 'task_completed' }),
		);
		expect(done.ok).toBe(true);
		const cleared = await client.getResponse(stale.value.record.id, 0);
		expect(cleared).toMatchObject({
			ok: true,
			value: { status: 'rescinded', rescindReason: 'handled_in_session' },
		});
		const result = await runTool(
			'wait',
			{ messageId: stale.value.record.id },
			{
				getClient: async () => succeed(client),
				sender: { ...TEST_SENDER, conversationId: IDENTITY.conversationId },
				defaultTimeoutSeconds: 5,
			},
			undefined,
		);
		expect(JSON.stringify(result)).toContain('not withdrawn');

		const explicit = await client.postMessage(
			message('Plan is ready', {
				type: 'generic',
				correlationKey: 'explicit-1',
			}),
		);
		if (!explicit.ok) throw new Error('post failed');
		const withdrawn = await client.rescindMessage(
			explicit.value.record.id,
			'resolved_elsewhere',
		);
		expect(withdrawn).toMatchObject({
			ok: true,
			value: { status: 'rescinded', rescindReason: 'resolved_elsewhere' },
		});
	});

	it('notify reports the mute to the model', async () => {
		const client = await connect();
		const posted = await client.postMessage(
			message('Release notes are drafted', {
				type: 'task_completed',
				event: 'integration.muted',
			}),
		);
		expect(
			posted.ok && posted.value.record.warnings?.map((warning) => warning.code),
		).toContain('muted');
	});

	it('rescinds by id and by correlation key', async () => {
		const client = await connect();
		const first = await client.postMessage(
			message('Plan is ready for review', {
				type: 'generic',
				correlationKey: 'plan-a',
			}),
		);
		if (!first.ok) throw new Error('post failed');
		const byKey = await client.rescindByCorrelationKeys({
			reason: 'obsolete',
			correlationKeys: ['plan-a'],
		});
		expect(byKey).toEqual({ ok: true, value: [first.value.record.id] });
		const second = await client.postMessage(
			message('Another plan is ready', { type: 'generic' }),
		);
		if (!second.ok) throw new Error('post failed');
		const byId = await client.rescindMessage(
			second.value.record.id,
			'obsolete',
		);
		expect(byId).toMatchObject({
			ok: true,
			value: { status: 'rescinded', rescindReason: 'obsolete' },
		});
	});

	it('the CLI sends a message end to end', async () => {
		let out = '';
		const code = await runCli(
			[
				'--agent',
				'claude-code',
				'--conversation',
				'cli-it',
				'send',
				'--type',
				'task_completed',
				'--title',
				'The CLI reached the hub',
			],
			{
				environment: environmentOf(['GENTBOX_API_DIR', apiDirectory]),
				workingDirectory: '/work/gentbox',
				writeOut: (text) => {
					out += text;
				},
				writeError: () => undefined,
				readStdin: async () => '',
			},
		);
		expect(code).toBe(0);
		expect(JSON.parse(out)).toMatchObject({
			type: 'task_completed',
			state: 'open',
		});
	});

	it('the MCP notify tool reports the hub warnings verbatim', async () => {
		const client = await connect();
		const result = await runTool(
			'notify',
			{ type: 'generic', title: 'git push origin main' },
			{
				getClient: async () => ({ ok: true, value: client }),
				sender: { ...TEST_SENDER, conversationId: IDENTITY.conversationId },
				defaultTimeoutSeconds: 5,
			},
			undefined,
		);
		const text = result.content
			.map((block) => (block.type === 'text' ? block.text : ''))
			.join('\n');
		expect(text).toContain('[title_looks_like_command] title:');
	});
});

async function waitUntil(condition: () => boolean): Promise<void> {
	const deadline = Date.now() + 20_000;
	while (!condition()) {
		if (Date.now() > deadline)
			throw new Error('gentboxd did not start in time');
		await new Promise((resolve) => setTimeout(resolve, 25));
	}
}
