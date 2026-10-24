import { chmodSync, mkdirSync, writeFileSync } from 'node:fs';
import { createServer, type IncomingHttpHeaders, type Server } from 'node:http';
import { join } from 'node:path';
import type { DiscoveryEnvironment } from '../discovery.ts';
import { computeHelloProof } from '../hub-client.ts';

export type RecordedRequest = {
	method: string;
	url: string;
	headers: IncomingHttpHeaders;
	body: string;
	transport: 'socket' | 'tcp';
};

export type FakeAnswer = {
	status: number;
	body?: unknown;
	headers?: Record<string, string>;
};

export type FakeHubOptions = {
	/** An empty, private folder; the fake writes `API/` into it. */
	directory: string;
	token: string;
	transports: ('socket' | 'tcp')[];
	/** Answers `hello` with a proof made from another token, like an impostor on a stale port. */
	isImpostor?: boolean;
	handler: (request: RecordedRequest) => FakeAnswer | Promise<FakeAnswer>;
};

export type FakeHub = {
	environment: DiscoveryEnvironment;
	requests: RecordedRequest[];
	port: number | undefined;
	/** From now on `hello` is answered with a proof for another token: the port was taken over. */
	becomeImpostor: () => void;
	close: () => Promise<void>;
};

/** A stand-in for gentboxd: the same discovery folder and `hello`, with canned answers. */
export async function startFakeHub(options: FakeHubOptions): Promise<FakeHub> {
	const apiDirectory = join(options.directory, 'API');
	mkdirSync(apiDirectory, { recursive: true, mode: 0o700 });
	chmodSync(apiDirectory, 0o700);
	writeFileSync(join(apiDirectory, 'token'), options.token, { mode: 0o600 });
	chmodSync(join(apiDirectory, 'token'), 0o600);

	const requests: RecordedRequest[] = [];
	const servers: Server[] = [];
	let tcpPort: number | undefined;
	let isImpostor = options.isImpostor === true;

	const serve = (transport: 'socket' | 'tcp') =>
		createServer((incoming, outgoing) => {
			const chunks: Buffer[] = [];
			incoming.on('data', (chunk: Buffer) => chunks.push(chunk));
			incoming.on('end', async () => {
				const recorded: RecordedRequest = {
					method: incoming.method ?? '',
					url: incoming.url ?? '',
					headers: incoming.headers,
					body: Buffer.concat(chunks).toString('utf8'),
					transport,
				};
				requests.push(recorded);
				const answer =
					recorded.url === '/v1/hello'
						? answerHello(recorded)
						: await options.handler(recorded);
				outgoing.writeHead(answer.status, {
					'content-type': 'application/json',
					...answer.headers,
				});
				outgoing.end(
					answer.body === undefined ? undefined : JSON.stringify(answer.body),
				);
			});
		});

	const answerHello = (request: RecordedRequest): FakeAnswer => {
		const { nonce } = JSON.parse(request.body);
		const key = isImpostor ? `${options.token}-impostor` : options.token;
		return {
			status: 200,
			body: {
				proof: computeHelloProof(key, tcpPort ?? 0, nonce),
				hub: 'gentbox',
				version: '0.0.0-fake',
				schemaVersions: [1],
			},
		};
	};

	if (options.transports.includes('tcp')) {
		const server = serve('tcp');
		servers.push(server);
		await new Promise<void>((resolve) =>
			server.listen(0, '127.0.0.1', resolve),
		);
		const address = server.address();
		if (address === null || typeof address === 'string')
			throw new Error('the fake hub did not bind a TCP port');
		tcpPort = address.port;
		writeFileSync(join(apiDirectory, 'port'), String(tcpPort), { mode: 0o600 });
	}
	if (options.transports.includes('socket')) {
		const server = serve('socket');
		servers.push(server);
		await new Promise<void>((resolve) =>
			server.listen(join(apiDirectory, 'hub.sock'), resolve),
		);
	}

	return {
		environment: {
			apiDirectoryOverride: apiDirectory,
			homeDirectory: options.directory,
			xdgStateHome: undefined,
		},
		requests,
		port: tcpPort,
		becomeImpostor: () => {
			isImpostor = true;
		},
		close: () =>
			Promise.all(
				servers.map(
					(server) =>
						new Promise<void>((resolve) => {
							server.close(() => resolve());
							server.closeAllConnections();
						}),
				),
			).then(() => undefined),
	};
}
