import { Ajv2020 } from 'ajv/dist/2020.js';
import { describe, expect, it } from 'vitest';
import { TOOL_SPECS, type ToolSpec } from './mcp-tool-schemas.ts';

function specOf(name: string): ToolSpec {
	const spec = TOOL_SPECS.find((candidate) => candidate.name === name);
	if (spec === undefined) throw new Error(`no tool ${name}`);
	return spec;
}

function jsonSchemaAccepts(spec: ToolSpec, input: unknown): boolean {
	return new Ajv2020({ strict: false }).compile(spec.inputSchema)(input);
}

describe('the tool list', () => {
	it('offers exactly the planned tools, each with an object input schema', () => {
		expect(TOOL_SPECS.map((spec) => spec.name)).toEqual([
			'notify',
			'ask',
			'rescind',
			'create_topic',
			'close_topic',
			'create_thread',
			'close_thread',
		]);
		for (const spec of TOOL_SPECS) expect(spec.inputSchema.type).toBe('object');
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
				expect(description).toMatch(/e\.g\. "[A-Z]/);
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
	])(
		'%s %j is accepted: %s, by the JSON Schema and by the validator alike',
		(name, input, isAccepted) => {
			const spec = specOf(name);
			expect(jsonSchemaAccepts(spec, input)).toBe(isAccepted);
			expect(spec.schema.safeParse(input).success).toBe(isAccepted);
		},
	);

	it('lists steps as required in the schema a client sees', () => {
		const allOf = JSON.stringify(specOf('notify').inputSchema.allOf);
		expect(allOf).toContain('"const":"action_required"');
		expect(allOf).toContain('"required":["steps"]');
	});
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
			spec.schema.safeParse({ messageId: 'a', correlationKeys: ['b'] }).success,
		).toBe(false);
		expect(spec.schema.safeParse({ messageId: 'a' }).success).toBe(true);
		expect(jsonSchemaAccepts(spec, {})).toBe(false);
		expect(jsonSchemaAccepts(spec, { correlationKeys: ['b'] })).toBe(true);
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
