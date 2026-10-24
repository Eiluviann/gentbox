/** A process environment from name/value pairs (upper-case names read as constants, not properties). */
export function environmentOf(
	...entries: [name: string, value: string][]
): NodeJS.ProcessEnv {
	return Object.fromEntries(entries);
}
