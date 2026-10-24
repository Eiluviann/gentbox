import {
	existsSync,
	lstatSync,
	readdirSync,
	readFileSync,
	statSync,
} from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { fail, type Result, succeed } from './result.ts';

/** Where the hub's API folder is searched (architecture §8). */
export type DiscoveryEnvironment = {
	/** Overrides the search: the API folder itself. */
	apiDirectoryOverride: string | undefined;
	homeDirectory: string;
	xdgStateHome: string | undefined;
};

/** What `<state>/API/` holds, read from disk. */
export type HubEndpoint = {
	directory: string;
	/** The sender token: bearer value and `hello` HMAC key. Never log it. */
	token: string;
	/** The Unix socket, when it exists: the preferred transport. */
	socketPath: string | undefined;
	/** The hub's TCP port, when the hub is serving it. */
	port: number | undefined;
};

const PAID_TEAM_GROUP = 'group.com.eiluviann.gentbox';
const PERSONAL_TEAM_GROUP_PATTERN = /^[A-Z0-9]+\.com\.eiluviann\.gentbox$/;
const TOKEN_PATTERN = /^[A-Za-z0-9_-]{16,128}$/;

/** The environment of this process. */
export function readDiscoveryEnvironment(
	environment: NodeJS.ProcessEnv = process.env,
	homeDirectory: string = homedir(),
): DiscoveryEnvironment {
	return {
		apiDirectoryOverride: nonEmpty(environment.GENTBOX_API_DIR),
		homeDirectory,
		xdgStateHome: nonEmpty(environment.XDG_STATE_HOME),
	};
}

/**
 * The folders to try, best first: `GENTBOX_API_DIR` alone when set; else the
 * App Group container (paid team, then a Personal Team's), then
 * `~/Library/Application Support/Gentbox/API`, then `$XDG_STATE_HOME/gentbox/API`.
 */
export function listApiDirectoryCandidates(
	environment: DiscoveryEnvironment,
): string[] {
	if (environment.apiDirectoryOverride !== undefined) {
		return [environment.apiDirectoryOverride];
	}
	const groupContainers = join(
		environment.homeDirectory,
		'Library/Group Containers',
	);
	const xdgStateHome =
		environment.xdgStateHome ?? join(environment.homeDirectory, '.local/state');
	return [
		join(groupContainers, PAID_TEAM_GROUP, 'API'),
		...listPersonalTeamContainers(groupContainers).map((container) =>
			join(container, 'API'),
		),
		join(environment.homeDirectory, 'Library/Application Support/Gentbox/API'),
		join(xdgStateHome, 'gentbox/API'),
	];
}

/**
 * Reads one API folder. It must be a real folder owned by this user that
 * nobody else can open, holding a token only this user can read: a token in a
 * shared place is not a secret.
 */
export function readHubEndpoint(
	directory: string,
): Result<HubEndpoint, string> {
	const folder = safeLstat(directory);
	if (folder === undefined) return fail(`${directory} does not exist`);
	if (folder.isSymbolicLink() || !folder.isDirectory()) {
		return fail(`${directory} must be a real folder, not a link`);
	}
	const tokenPath = join(directory, 'token');
	const tokenFile = safeLstat(tokenPath);
	if (tokenFile === undefined) return fail(`${tokenPath} does not exist`);
	const owner = process.getuid?.();
	const isOwnedByAnotherUser =
		owner !== undefined && (folder.uid !== owner || tokenFile.uid !== owner);
	if (isOwnedByAnotherUser)
		return fail(`${directory} is not owned by the current user`);
	const isOpenToOthers =
		(folder.mode & 0o077) !== 0 || (tokenFile.mode & 0o077) !== 0;
	if (isOpenToOthers)
		return fail(
			`${directory} or its token is readable by other users; refusing to use it`,
		);
	if (tokenFile.isSymbolicLink() || !tokenFile.isFile())
		return fail(`${tokenPath} must be a regular file`);
	const token = readTrimmed(tokenPath);
	if (token === undefined || !TOKEN_PATTERN.test(token))
		return fail(`${tokenPath} does not hold a token`);
	const socketPath = join(directory, 'hub.sock');
	return succeed({
		directory,
		token,
		socketPath: isSocket(socketPath) ? socketPath : undefined,
		port: readPort(join(directory, 'port')),
	});
}

/** Every readable endpoint in search order, and why each skipped folder was skipped. */
export function discoverHubEndpoints(environment: DiscoveryEnvironment): {
	endpoints: HubEndpoint[];
	skipped: string[];
} {
	const endpoints: HubEndpoint[] = [];
	const skipped: string[] = [];
	for (const directory of listApiDirectoryCandidates(environment)) {
		const endpoint = readHubEndpoint(directory);
		if (endpoint.ok) endpoints.push(endpoint.value);
		else skipped.push(endpoint.error);
	}
	return { endpoints, skipped };
}

function listPersonalTeamContainers(groupContainers: string): string[] {
	try {
		return readdirSync(groupContainers)
			.filter((name) => PERSONAL_TEAM_GROUP_PATTERN.test(name))
			.sort()
			.map((name) => join(groupContainers, name));
	} catch {
		return [];
	}
}

function safeLstat(path: string) {
	try {
		return lstatSync(path);
	} catch {
		return undefined;
	}
}

function isSocket(path: string): boolean {
	try {
		return existsSync(path) && statSync(path).isSocket();
	} catch {
		return false;
	}
}

function readTrimmed(path: string): string | undefined {
	try {
		return readFileSync(path, 'utf8').trim();
	} catch {
		return undefined;
	}
}

function readPort(path: string): number | undefined {
	const text = readTrimmed(path);
	if (text === undefined || !/^[0-9]{1,5}$/.test(text)) return undefined;
	const port = Number(text);
	return port >= 1 && port <= 65535 ? port : undefined;
}

function nonEmpty(value: string | undefined): string | undefined {
	return value === undefined || value === '' ? undefined : value;
}
