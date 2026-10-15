/*
 * The helpers of the script editor keep the order of the scripts, convert a draft to the API shape
 * and find the hosts that cannot run a script (FR-HK-09, FR-HK-10, ADR-0037).
 */
import { describe, expect, it } from 'vitest';
import type { Agent, ScriptTemplate } from '#lib/api/index.ts';
import {
	blankScript,
	canMove,
	draftFromTemplate,
	draftOf,
	enableCommand,
	groupOf,
	hostProblem,
	hostsWithProblems,
	interpretersFor,
	move,
	scriptOf
} from './scripts';

const TEMPLATE: ScriptTemplate = {
	id: 'builtin:postgres-dump-bash',
	builtin: true,
	name: 'PostgreSQL dump',
	description: 'Dumps one database.',
	interpreter: 'bash',
	phase: 'before',
	body: 'pg_dump "$PGDATABASE"\n',
	timeout_seconds: 600,
	failure_policy: 'abort_backup',
	parameters: [
		{ name: 'PGDATABASE', label: 'Database', type: 'string', default: 'app' },
		{ name: 'PGPASSWORD', label: 'Password', type: 'secret_ref' },
		{ name: 'PGPORT', label: 'Port', type: 'integer', default: '5432' }
	]
};

function agent(overrides: Partial<Agent> = {}): Agent {
	return {
		id: 'a_1',
		hostname: 'srv-db-01',
		os: 'linux',
		arch: 'amd64',
		agent_version: '0.4.2',
		restic_version: '0.19.0',
		labels: [],
		restore_roots: [],
		hook_allow_list: 'absent',
		hooks: [],
		server_scripts: 'enabled',
		script_interpreters: ['bash', 'sh'],
		certificate_serial: '01',
		certificate_expires_at: '2999-01-01T00:00:00Z',
		status: 'active',
		last_seen_at: null,
		created_at: '2026-01-01T00:00:00Z',
		revoked_at: null,
		...overrides
	} as Agent;
}

describe('script drafts', () => {
	it('puts the phase `before` in the first list and every other phase in the second', () => {
		expect(groupOf('before')).toBe('before');
		for (const phase of ['after', 'on_success', 'on_error', 'finally'] as const) {
			expect(groupOf(phase)).toBe('after');
		}
		expect(blankScript('before').phase).toBe('before');
		expect(blankScript('after').phase).toBe('after');
	});

	it('turns the parameters of a template into variables with their defaults', () => {
		const draft = draftFromTemplate(TEMPLATE);
		expect(draft.env).toEqual([
			{ name: 'PGDATABASE', kind: 'string', value: 'app' },
			{ name: 'PGPASSWORD', kind: 'secret_ref', value: '' },
			{ name: 'PGPORT', kind: 'string', value: '5432' }
		]);
		expect(draft.templateId).toBe('builtin:postgres-dump-bash');
		expect(draft.timeoutSeconds).toBe('600');
	});

	it('round-trips a stored script through a draft', () => {
		const stored = scriptOf(draftFromTemplate(TEMPLATE));
		expect(scriptOf(draftOf(stored))).toEqual(stored);
		expect(stored.timeout_seconds).toBe(600);
		expect(stored.template_id).toBe('builtin:postgres-dump-bash');
	});

	it('drops a variable without a name and an empty time limit', () => {
		const draft = blankScript('before');
		draft.name = '  dump  ';
		draft.body = 'echo';
		draft.env = [
			{ name: '', kind: 'string', value: 'x' },
			{ name: ' A ', kind: 'string', value: '1' }
		];
		const script = scriptOf(draft);
		expect(script.name).toBe('dump');
		expect(script.timeout_seconds).toBe(0);
		expect(script.env).toEqual([{ name: 'A', kind: 'string', value: '1' }]);
		expect('template_id' in script).toBe(false);
	});
});

describe('script order', () => {
	function list() {
		const first = blankScript('before');
		const second = blankScript('before');
		const after = blankScript('after');
		const third = blankScript('before');
		return { first, second, after, third, all: [first, after, second, third] };
	}

	it('moves a script over its neighbour of the same list and skips the other list', () => {
		const { first, second, after, third, all } = list();
		const down = move(all, first.key, 1);
		expect(down.map((item) => item.key)).toEqual([second.key, after.key, first.key, third.key]);
		const up = move(all, third.key, -1);
		expect(up.map((item) => item.key)).toEqual([first.key, after.key, third.key, second.key]);
	});

	it('does not move a script at the edge of its list', () => {
		const { first, third, after, all } = list();
		expect(canMove(all, first.key, -1)).toBe(false);
		expect(canMove(all, third.key, 1)).toBe(false);
		expect(canMove(all, after.key, -1)).toBe(false);
		expect(canMove(all, after.key, 1)).toBe(false);
		expect(move(all, first.key, -1)).toBe(all);
	});
});

describe('hosts', () => {
	it('finds a host without the trust mode or without the interpreter', () => {
		expect(hostProblem(agent(), 'bash')).toBeNull();
		expect(hostProblem(agent({ server_scripts: 'disabled' }), 'bash')).toBe('disabled');
		expect(hostProblem(agent({ server_scripts: 'insecure' }), 'bash')).toBe('disabled');
		expect(hostProblem(agent(), 'powershell')).toBe('interpreter');
	});

	it('lists the active hosts that cannot run one of the scripts', () => {
		const scripts = [blankScript('before')];
		const hosts = [
			agent({ id: 'a_1', hostname: 'ok' }),
			agent({ id: 'a_2', hostname: 'off', server_scripts: 'disabled' }),
			agent({ id: 'a_3', hostname: 'revoked', server_scripts: 'disabled', status: 'revoked' })
		];
		expect(hostsWithProblems(hosts, scripts).map((host) => host.hostname)).toEqual(['off']);
		expect(hostsWithProblems(hosts, [])).toEqual([]);
	});

	it('gives the command for the operating system', () => {
		expect(enableCommand('linux')).toBe('sudo restichub-agent trust-server-scripts --enable');
		expect(enableCommand('Windows')).toBe('restichub-agent.exe trust-server-scripts --enable');
		expect(interpretersFor('windows')).toEqual(['powershell', 'cmd']);
		expect(interpretersFor('linux')).toEqual(['bash', 'sh']);
	});
});
