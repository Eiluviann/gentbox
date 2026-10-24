#!/usr/bin/env node
import { runCli } from '../main.ts';

async function readStandardInput(): Promise<string> {
	const chunks: Buffer[] = [];
	for await (const chunk of process.stdin) chunks.push(Buffer.from(chunk));
	return Buffer.concat(chunks).toString('utf8');
}

process.exitCode = await runCli(process.argv.slice(2), {
	environment: process.env,
	workingDirectory: process.cwd(),
	writeOut: (text) => process.stdout.write(text),
	writeError: (text) => process.stderr.write(text),
	readStdin: readStandardInput,
});
