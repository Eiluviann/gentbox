import {
	chmodSync,
	mkdirSync,
	mkdtempSync,
	rmSync,
	symlinkSync,
	writeFileSync,
} from 'node:fs';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import {
	type DiscoveryEnvironment,
	discoverHubEndpoints,
	listApiDirectoryCandidates,
	readDiscoveryEnvironment,
	readHubEndpoint,
} from './discovery.ts';
import { environmentOf } from './test-support/environment-of.ts';

const TOKEN = 'unit-test-token-'.padEnd(43, '0');

let home: string;

beforeEach(() => {
	home = mkdtempSync(join(tmpdir(), 'gentbox-discovery-'));
});

afterEach(() => {
	rmSync(home, { recursive: true, force: true });
});

function environmentFor(
	overrides: Partial<DiscoveryEnvironment> = {},
): DiscoveryEnvironment {
	return {
		apiDirectoryOverride: undefined,
		homeDirectory: home,
		xdgStateHome: undefined,
		...overrides,
	};
}

function writeApiDirectory(
	directory: string,
	files: { port?: string; token?: string } = {},
): string {
	mkdirSync(directory, { recursive: true, mode: 0o700 });
	chmodSync(directory, 0o700);
	const tokenPath = join(directory, 'token');
	writeFileSync(tokenPath, files.token ?? TOKEN, { mode: 0o600 });
	chmodSync(tokenPath, 0o600);
	if (files.port !== undefined)
		writeFileSync(join(directory, 'port'), files.port, { mode: 0o600 });
	return directory;
}

describe('listApiDirectoryCandidates', () => {
	it('searches the App Group containers, then Application Support, then the XDG state folder', () => {
		mkdirSync(
			join(home, 'Library/Group Containers/ABCDE12345.com.eiluviann.gentbox'),
			{ recursive: true },
		);
		mkdirSync(join(home, 'Library/Group Containers/group.com.other.app'), {
			recursive: true,
		});
		expect(listApiDirectoryCandidates(environmentFor())).toEqual([
			join(home, 'Library/Group Containers/group.com.eiluviann.gentbox/API'),
			join(
				home,
				'Library/Group Containers/ABCDE12345.com.eiluviann.gentbox/API',
			),
			join(home, 'Library/Application Support/Gentbox/API'),
			join(home, '.local/state/gentbox/API'),
		]);
	});

	it('honours XDG_STATE_HOME', () => {
		const candidates = listApiDirectoryCandidates(
			environmentFor({ xdgStateHome: '/state' }),
		);
		expect(candidates.at(-1)).toBe('/state/gentbox/API');
	});

	it('uses GENTBOX_API_DIR alone when it is set', () => {
		const candidates = listApiDirectoryCandidates(
			environmentFor({ apiDirectoryOverride: '/somewhere/API' }),
		);
		expect(candidates).toEqual(['/somewhere/API']);
	});

	it('reads the override and the XDG folder from the process environment', () => {
		const environment = readDiscoveryEnvironment(
			environmentOf(['GENTBOX_API_DIR', '/x'], ['XDG_STATE_HOME', '']),
			'/home/me',
		);
		expect(environment).toEqual({
			apiDirectoryOverride: '/x',
			homeDirectory: '/home/me',
			xdgStateHome: undefined,
		});
	});
});

describe('readHubEndpoint', () => {
	it('reads the token and the port, and finds no socket when there is none', () => {
		const directory = writeApiDirectory(join(home, 'API'), { port: '4242\n' });
		expect(readHubEndpoint(directory)).toEqual({
			ok: true,
			value: { directory, token: TOKEN, socketPath: undefined, port: 4242 },
		});
	});

	it('reports the socket only when the file is a socket', async () => {
		const directory = writeApiDirectory(join(home, 'API'));
		writeFileSync(join(directory, 'hub.sock'), '');
		expect(readHubEndpoint(directory)).toMatchObject({
			ok: true,
			value: { socketPath: undefined },
		});
		rmSync(join(directory, 'hub.sock'));
		const server = createServer();
		await new Promise<void>((resolve) =>
			server.listen(join(directory, 'hub.sock'), resolve),
		);
		try {
			expect(readHubEndpoint(directory)).toMatchObject({
				ok: true,
				value: { socketPath: join(directory, 'hub.sock') },
			});
		} finally {
			server.close();
		}
	});

	it('ignores a port file that is not a port', () => {
		const directory = writeApiDirectory(join(home, 'API'), { port: '99999' });
		expect(readHubEndpoint(directory)).toMatchObject({
			ok: true,
			value: { port: undefined },
		});
	});

	it('refuses a missing folder, a missing token and a malformed token', () => {
		expect(readHubEndpoint(join(home, 'nothing')).ok).toBe(false);
		mkdirSync(join(home, 'empty'), { mode: 0o700 });
		expect(readHubEndpoint(join(home, 'empty')).ok).toBe(false);
		const malformed = writeApiDirectory(join(home, 'malformed'), {
			token: 'short token',
		});
		expect(readHubEndpoint(malformed).ok).toBe(false);
	});

	it('refuses a token that other users can read', () => {
		const directory = writeApiDirectory(join(home, 'API'));
		chmodSync(join(directory, 'token'), 0o644);
		const endpoint = readHubEndpoint(directory);
		expect(endpoint).toMatchObject({ ok: false });
		expect(JSON.stringify(endpoint)).not.toContain(TOKEN);
	});

	it('refuses a folder that other users can open', () => {
		const directory = writeApiDirectory(join(home, 'API'));
		chmodSync(directory, 0o755);
		expect(readHubEndpoint(directory).ok).toBe(false);
	});

	it('refuses a folder that is a symlink', () => {
		const real = writeApiDirectory(join(home, 'real'));
		symlinkSync(real, join(home, 'link'));
		expect(readHubEndpoint(join(home, 'link')).ok).toBe(false);
	});
});

describe('discoverHubEndpoints', () => {
	it('returns the usable endpoints in search order and names the skipped folders', () => {
		const supportDirectory = writeApiDirectory(
			join(home, 'Library/Application Support/Gentbox/API'),
		);
		const groupDirectory = writeApiDirectory(
			join(home, 'Library/Group Containers/group.com.eiluviann.gentbox/API'),
		);
		const { endpoints, skipped } = discoverHubEndpoints(environmentFor());
		expect(endpoints.map((endpoint) => endpoint.directory)).toEqual([
			groupDirectory,
			supportDirectory,
		]);
		expect(skipped).toHaveLength(1);
	});
});
