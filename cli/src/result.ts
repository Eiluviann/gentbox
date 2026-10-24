/** An expected outcome of valid use: the caller checks `ok` (programmer errors throw instead). */
export type Result<T, E = Error> =
	| { ok: true; value: T }
	| { ok: false; error: E };

export function succeed<T>(value: T): { ok: true; value: T } {
	return { ok: true, value };
}

export function fail<E>(error: E): { ok: false; error: E } {
	return { ok: false, error };
}
