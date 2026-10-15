/*
 * AC-UI-01: every API call of the UI maps to an operation of the OpenAPI document.
 *
 * The test calls each method of the HTTP service with a recording `fetch` and compares the method
 * and the path of every request with the `paths` of the generated `schema.d.ts`. A method of the
 * service that sends no request fails the test, so a new method cannot skip the check.
 */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';

interface Call {
	method: string;
	path: string;
}

const API_PREFIX = '/api/v1';
const ID = '0190a000-0000-7000-8000-000000000001';

/** The `{method, path template}` pairs of the committed schema. */
function documentedOperations(): Set<string> {
	const schema = readFileSync(join(process.cwd(), 'src/lib/api/schema.d.ts'), 'utf8');
	const paths = schema.slice(
		schema.indexOf('export interface paths'),
		schema.indexOf('export type webhooks')
	);
	const operations = new Set<string>();
	let current: string | null = null;
	for (const line of paths.split('\n')) {
		const path = /^ {4}"([^"]+)": \{$/.exec(line);
		if (path) {
			current = path[1];
			continue;
		}
		const method = /^ {8}(get|put|post|delete|patch): operations\[/.exec(line);
		if (method && current) operations.add(`${method[1].toUpperCase()} ${current}`);
	}
	return operations;
}

function matches(template: string, path: string): boolean {
	const pattern = template
		.split('/')
		.map((part) => (part.startsWith('{') ? '[^/]+' : part.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')))
		.join('/');
	return new RegExp(`^${pattern}$`).test(path);
}

async function recordedCalls(): Promise<{ calls: Map<string, Call[]>; names: string[] }> {
	const calls = new Map<string, Call[]>();
	let current = '';
	vi.resetModules();
	vi.stubGlobal(
		'fetch',
		vi.fn(async (input: string, init?: RequestInit) => {
			const url = new URL(input, 'http://localhost');
			const list = calls.get(current) ?? [];
			list.push({ method: (init?.method ?? 'GET').toUpperCase(), path: url.pathname });
			calls.set(current, list);
			return new Response('{}', { status: 200, headers: { 'content-type': 'application/json' } });
		})
	);
	const service = (await import('./service')).service as unknown as Record<
		string,
		(...args: unknown[]) => Promise<unknown>
	>;
	const names = Object.keys(service);
	for (const name of names) {
		current = name;
		const attempts: unknown[][] = [
			[ID, {}, {}],
			[{}, {}],
			[ID, ID, {}]
		];
		for (const args of attempts) {
			try {
				await service[name](...args);
			} catch {
				// A response of `{}` may break the unwrap of one method. The request was sent first.
			}
			if (calls.has(name)) break;
		}
	}
	return { calls, names };
}

afterEach(() => {
	vi.unstubAllGlobals();
});

describe('AC-UI-01 the endpoint coverage', () => {
	it('ac_ui_01_every_api_call_of_the_ui_maps_to_an_openapi_operation', async () => {
		const documented = documentedOperations();
		expect(documented.size).toBeGreaterThan(50);
		const { calls, names } = await recordedCalls();

		const silent = names.filter((name) => !calls.has(name));
		expect(silent, 'service methods that sent no request').toEqual([]);

		const unmapped: string[] = [];
		let checked = 0;
		for (const [name, list] of calls) {
			for (const call of list) {
				checked += 1;
				expect(call.path.startsWith(API_PREFIX), `${name} leaves ${API_PREFIX}`).toBe(true);
				const found = [...documented].some((operation) => {
					const [method, template] = operation.split(' ');
					return method === call.method && matches(template, call.path);
				});
				if (!found) unmapped.push(`${name}: ${call.method} ${call.path}`);
			}
		}
		expect(unmapped, 'calls with no OpenAPI operation').toEqual([]);
		expect(checked).toBeGreaterThanOrEqual(names.length);
	});

	it('ac_ui_01_the_endpoint_table_holds_only_documented_paths', async () => {
		const { endpoints } = await import('./endpoints');
		const documented = [...documentedOperations()].map((operation) => operation.split(' ')[1]);
		const paths: string[] = [];
		const walk = (value: unknown): void => {
			if (typeof value === 'string') paths.push(value);
			else if (typeof value === 'function')
				paths.push((value as (...a: unknown[]) => string)(ID, ID, 1));
			else if (value && typeof value === 'object') Object.values(value).forEach(walk);
		};
		walk(endpoints);
		const missing = paths
			.map((path) => `${API_PREFIX}${path.split('?')[0]}`)
			.filter((path) => !documented.some((template) => matches(template, path)));
		expect(missing).toEqual([]);
	});
});
