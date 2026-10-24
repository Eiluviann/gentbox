import { request as httpRequest, type IncomingHttpHeaders } from 'node:http';
import { fail, type Result, succeed } from './result.ts';

/** Where a request goes: the hub's Unix socket or its loopback TCP port. */
export type HubTarget =
	| { kind: 'socket'; socketPath: string }
	| { kind: 'tcp'; port: number };

export type RawRequest = {
	method: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';
	path: string;
	headers: Record<string, string>;
	body: string | undefined;
	timeoutMilliseconds: number;
	signal: AbortSignal | undefined;
};

export type RawResponse = {
	status: number;
	headers: IncomingHttpHeaders;
	body: string;
};

/** Sends one request. A refused connection, a timeout or an abort is a failure message, never a throw. */
export type HttpTransport = (
	target: HubTarget,
	request: RawRequest,
) => Promise<Result<RawResponse, string>>;

/** The real transport: `node:http` over the socket or `127.0.0.1`. */
export const sendHttpRequest: HttpTransport = (target, request) =>
	new Promise((resolve) => {
		const headers: Record<string, string> = { ...request.headers };
		if (target.kind === 'socket') headers.host = 'localhost';
		if (request.body !== undefined) {
			headers['content-type'] = 'application/json';
			headers['content-length'] = String(Buffer.byteLength(request.body));
		}
		const options =
			target.kind === 'socket'
				? { socketPath: target.socketPath }
				: { host: '127.0.0.1', port: target.port };
		const outgoing = httpRequest(
			{
				...options,
				method: request.method,
				path: request.path,
				headers,
				signal: request.signal,
			},
			(incoming) => {
				const chunks: Buffer[] = [];
				incoming.on('data', (chunk: Buffer) => chunks.push(chunk));
				incoming.on('error', (error) => resolve(fail(describe(error))));
				incoming.on('end', () =>
					resolve(
						succeed({
							status: incoming.statusCode ?? 0,
							headers: incoming.headers,
							body: Buffer.concat(chunks).toString('utf8'),
						}),
					),
				);
			},
		);
		outgoing.setTimeout(request.timeoutMilliseconds, () => {
			outgoing.destroy(
				new Error(`no answer within ${request.timeoutMilliseconds} ms`),
			);
		});
		outgoing.on('error', (error) => resolve(fail(describe(error))));
		outgoing.end(request.body);
	});

function describe(error: unknown): string {
	if (error instanceof Error) {
		const code =
			'code' in error && typeof error.code === 'string'
				? ` (${error.code})`
				: '';
		return `${error.message}${code}`;
	}
	return String(error);
}
