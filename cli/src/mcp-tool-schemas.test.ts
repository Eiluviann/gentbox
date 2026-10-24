import { describe, expect, it } from 'vitest';
import { TOOL_SPECS, type ToolSpec } from './mcp-tool-schemas.ts';

function specOf(name: string): ToolSpec {
	const spec = TOOL_SPECS.find((candidate) => candidate.name === name);
	if (spec === undefined) throw new Error(`no tool ${name}`);
	return spec;
}

/** The `description` of a property in a JSON Schema. */
function descriptionOf(property: unknown): string {
	if (
		typeof property !== 'object' ||
		property === null ||
		!('description' in property)
	)
		return '';
	return typeof property.description === 'string' ? property.description : '';
}

describe('the tool list', () => {
	it('offers exactly the planned tools, each with an object input schema', () => {
		expect(TOOL_SPECS.map((spec) => spec.name)).toEqual([
			'notify',
			'ask',
			'wait',
			'rescind',
			'create_topic',
			'close_topic',
			'create_thread',
			'close_thread',
		]);
		for (const spec of TOOL_SPECS) expect(spec.inputSchema.type).toBe('object');
	});

	it("never promises to withdraw a message on the model's behalf", () => {
		const askText = specOf('ask').description;
		expect(askText).not.toContain('is withdrawn');
		expect(askText).not.toContain('the message is withdrawn');
		expect(askText).toContain('stays open');
		expect(askText).toContain('muted');
		expect(askText).toContain('`wait`');
		expect(askText).toContain('`rescind`');
		const timeoutText = descriptionOf(
			specOf('ask').inputSchema.properties?.timeoutSeconds,
		);
		expect(timeoutText).toContain('stays open');
		expect(timeoutText).not.toContain('withdrawn');
	});

	it('tells the model a muted notification still arrives, silently', () => {
		const notifyText = specOf('notify').description;
		expect(notifyText).toContain('muted');
		expect(notifyText).toContain('inbox');
	});

	it('states that withdrawn means the agent asked', () => {
		expect(specOf('rescind').description).toContain(
			'Withdrawn means you asked',
		);
		expect(specOf('wait').description).not.toContain('is withdrawn');
	});

	it.each(['notify', 'ask', 'create_topic', 'create_thread'])(
		'%s states the title rule with a good and a bad example',
		(name) => {
			const description = specOf(name).description;
			if (name === 'notify' || name === 'ask') {
				expect(description).toContain(
					'never a command, path, URL, code or raw tool input',
				);
				expect(description).toContain(
					'Good: "Allow Bash to run the ingest tests?"',
				);
				expect(description).toContain(
					'Bad: "cargo test -p gentbox-ingest -- --nocapture"',
				);
			} else {
				expect(description).toContain('never a command or path');
				expect(description).toMatch(/Good: "[A-Z][^"]+"/);
				expect(description).toContain('Bad: "npm publish --tag next"');
			}
		},
	);

	it.each(['notify', 'ask'])(
		'%s requires steps for action_required, with an example',
		(name) => {
			const description = specOf(name).description;
			expect(description).toContain('`steps` is required');
			expect(description).toContain('"text":"Sign in to npm"');
		},
	);

	it.each([
		['create_topic', 'Good: "Release 1.4"'],
		['create_thread', 'Good: "Phase 3 ingest"'],
	])(
		'%s: the title parameter carries a good and a bad example',
		(name, good) => {
			const title = descriptionOf(specOf(name).inputSchema.properties?.title);
			expect(title).toContain(good);
			expect(title).toContain('Bad: "npm publish --tag next"');
		},
	);

	it('the title parameter says what a title is', () => {
		const properties = specOf('notify').inputSchema.properties;
		expect(JSON.stringify(properties?.title)).toContain(
			'never a command, path or URL',
		);
	});
});

describe('the conditional requirement of steps', () => {
	const base = { title: 'Sign in to npm so I can publish the CLI package' };
	const steps = [{ text: 'Open a terminal in any folder' }];

	it.each([
		['notify', { ...base, type: 'action_required' }, false],
		['notify', { ...base, type: 'action_required', steps }, true],
		['notify', { ...base, type: 'task_completed', steps }, false],
		['notify', { ...base, type: 'task_completed' }, true],
		['notify', { ...base }, false],
		['ask', { ...base, type: 'action_required' }, false],
		['ask', { ...base, type: 'action_required', steps }, true],
		['ask', { ...base }, true],
		['ask', { ...base, steps }, false],
	])('%s %j is accepted: %s', (name, input, isAccepted) => {
		expect(specOf(name).schema.safeParse(input).success).toBe(isAccepted);
	});

	it('names the missing field in the validation error', () => {
		const parsed = specOf('notify').schema.safeParse({
			...base,
			type: 'action_required',
		});
		expect(parsed.success).toBe(false);
		expect(parsed.error?.issues[0]).toMatchObject({
			path: ['steps'],
			message: '`steps` is required when `type` is `action_required`',
		});
	});

	it.each(['notify', 'ask'])(
		'%s says in the steps field that action_required needs it',
		(name) => {
			const steps = specOf(name).inputSchema.properties?.steps;
			expect(descriptionOf(steps)).toContain(
				'required when type is action_required',
			);
		},
	);
});

describe('the root of every input schema', () => {
	// The Claude API answers 400 "input_schema does not support oneOf, allOf, or anyOf at the top level".
	it.each(TOOL_SPECS.map((spec) => spec.name))(
		'%s is a plain object schema',
		(name) => {
			const root = specOf(name).inputSchema;
			expect(root.type).toBe('object');
			for (const keyword of [
				'allOf',
				'anyOf',
				'oneOf',
				'not',
				'if',
				'then',
				'else',
				'enum',
			]) {
				expect(root, `${name} uses ${keyword} at the root`).not.toHaveProperty(
					keyword,
				);
			}
		},
	);
});

describe('argument validation', () => {
	it('rejects unknown arguments so a typo is not silently ignored', () => {
		expect(
			specOf('notify').schema.safeParse({
				type: 'generic',
				title: 'x',
				urgnt: true,
			}).success,
		).toBe(false);
	});

	it('rescind needs exactly one of messageId and correlationKeys', () => {
		const spec = specOf('rescind');
		expect(spec.schema.safeParse({}).success).toBe(false);
		expect(
			spec.schema.safeParse({
				messageId: '01JBMA2W8R6T4Y2V0P8N6M4K2H',
				correlationKeys: ['b'],
			}).success,
		).toBe(false);
		expect(spec.schema.safeParse({ messageId: '../topics' }).success).toBe(
			false,
		);
		expect(
			spec.schema.safeParse({ messageId: '01JBMA2W8R6T4Y2V0P8N6M4K2H' })
				.success,
		).toBe(true);
		expect(spec.schema.safeParse({ correlationKeys: ['b'] }).success).toBe(
			true,
		);
		expect(spec.description).toContain(
			'Give exactly one of `messageId` or `correlationKeys`',
		);
		expect(spec.description).toContain('or by `correlationKeys`');
	});

	it('ask offers blocking and deferrable only', () => {
		const spec = specOf('ask');
		expect(
			spec.schema.safeParse({ title: 'x', blocking: 'non_blocking' }).success,
		).toBe(false);
		expect(
			spec.schema.safeParse({ title: 'x', blocking: 'deferrable' }).success,
		).toBe(true);
	});
});
