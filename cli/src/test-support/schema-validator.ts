import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Ajv2020 } from 'ajv/dist/2020.js';
import addFormats from 'ajv-formats';

export const SCHEMA_DIRECTORY = join(
	dirname(fileURLToPath(import.meta.url)),
	'../../../docs/schema',
);
const SCHEMA_ID =
	'https://github.com/Eiluviann/gentbox/docs/schema/message-v1.json';

export function readJson(path: string): unknown {
	return JSON.parse(readFileSync(path, 'utf8'));
}

/** An Ajv validator for docs/schema/message-v1.json itself (a message) or one of its `$defs`. */
export function createSchemaValidator(definition: string | undefined) {
	const ajv = new Ajv2020({ strict: false, allErrors: true });
	addFormats.default(ajv);
	ajv.addSchema(readJson(join(SCHEMA_DIRECTORY, 'message-v1.json')) ?? {});
	const reference =
		definition === undefined ? SCHEMA_ID : `${SCHEMA_ID}#/$defs/${definition}`;
	return ajv.compile({ $ref: reference });
}

/** Whether `value`, as the hub would receive it over JSON, satisfies the message schema. */
export function isValidMessageJson(value: unknown): {
	isValid: boolean;
	errors: string;
} {
	const validate = createSchemaValidator(undefined);
	const isValid = validate(JSON.parse(JSON.stringify(value)));
	return { isValid, errors: JSON.stringify(validate.errors) };
}
