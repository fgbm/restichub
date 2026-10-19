/*
 * The variables of a profile template: the expression at the caret, the groups of the list, the
 * filter, the date formats and the label keys (FR-PF-03, FR-PF-10, AC-PF-03, AC-PF-09).
 */
import { describe, expect, it } from 'vitest';
import {
	HOST_VARIABLES,
	caretExpression,
	dateExample,
	hostLabels,
	secretReference,
	variableCount,
	variableGroups
} from './variables';
import type { VariableSource } from './variables';

const NOW = new Date('2026-10-19T09:42:00Z');

const source: VariableSource = {
	now: NOW,
	parameters: [
		{ name: 'pg_password', kind: 'secret', description: 'Backup password' },
		{ name: 'pg_dir', kind: 'text', description: 'Directory of the dumps' },
		{ name: 'keep_logs', kind: 'text', description: null },
		{ name: 'pg_host', kind: 'text', description: 'Address of the database' }
	],
	labels: [
		{ key: 'env', value: 'prod' },
		{ key: 'role', value: 'db' }
	]
};

const flat = (typed: string, from: VariableSource = source) =>
	variableGroups(typed, from).flatMap((group) => group.options);

describe('the expression at the caret', () => {
	it('fr_pf_03_opens_only_after_two_braces', () => {
		expect(caretExpression('/srv/{{', 7)).toEqual({ start: 5, typed: '' });
		expect(caretExpression('/srv/{{pg', 9)).toEqual({ start: 5, typed: 'pg' });
		expect(caretExpression('{{agent.label.', 14)).toEqual({ start: 0, typed: 'agent.label.' });
		expect(caretExpression('/srv/{{date:%Y', 14)).toEqual({ start: 5, typed: 'date:%Y' });
	});

	it('fr_pf_03_stays_closed_outside_the_expression', () => {
		expect(caretExpression('/srv/postgres', 14)).toBeNull();
		// A space before `{{` marks a text and not a variable.
		expect(caretExpression('/srv {{pg', 9)).toBeNull();
		// A closed expression is a ready reference, not a text to complete.
		expect(caretExpression('{{param.pg_dir}}', 16)).toBeNull();
		expect(caretExpression('{{pg\n/etc', 7)).toBeNull();
		expect(caretExpression('{{', 1)).toBeNull();
	});
});

describe('the groups of the list', () => {
	it('fr_pf_10_shows_the_parameters_above_the_host_variables', () => {
		const groups = variableGroups('', source);
		expect(groups.map((group) => group.id)).toEqual(['parameters', 'host']);
		expect(groups[0].options.map((option) => option.label)).toEqual([
			'pg_password',
			'pg_dir',
			'keep_logs',
			'pg_host'
		]);
		expect(groups[1].options.map((option) => option.expression)).toEqual([
			...HOST_VARIABLES.slice(0, 3),
			'agent.label.',
			...HOST_VARIABLES.slice(3)
		]);
	});

	it('fr_pf_03_names_the_built_in_variables_with_an_example', () => {
		const options = flat('');
		expect(options.find((option) => option.expression === 'hostname')?.description).toContain(
			'db-01.example.com'
		);
		expect(options.find((option) => option.expression === 'date')?.description).toContain(
			'2026-10-19'
		);
		expect(options.find((option) => option.expression === 'date:%Y%m%d')?.description).toContain(
			'20261019'
		);
		expect(options.find((option) => option.expression === 'agent.label.')?.note).toBe('type a key');
		expect(options.find((option) => option.expression === 'agent.label.')?.partial).toBe(true);
	});

	it('fr_pf_10_filters_by_the_typed_text', () => {
		const options = flat('pg');
		expect(options.map((option) => option.label)).toEqual(['pg_password', 'pg_dir', 'pg_host']);
		expect(variableGroups('pg', source).map((group) => group.id)).toEqual(['parameters']);
	});

	it('fr_pf_10_keeps_a_secret_in_the_list_and_disables_it', () => {
		const secret = flat('pg').find((option) => option.label === 'pg_password');
		expect(secret?.kind).toBe('secret');
		expect(secret?.disabled).toBe(true);
		expect(secret?.reason).toBe('A secret cannot go into a path or a tag.');
	});

	it('fr_pf_03_offers_a_free_date_format', () => {
		const free = flat('date:%Y-%m');
		expect(free.map((option) => option.expression)).toEqual(['date:%Y-%m']);
		expect(free[0].description).toContain('2026-10');
		// An unknown directive is not a format, so the group has no row.
		expect(flat('date:%Q')).toEqual([]);
		// The fixed example stays the only row until the format has a character.
		expect(flat('date:').map((option) => option.expression)).toEqual(['date:%Y%m%d']);
	});
});

describe('the labels of the hosts', () => {
	it('fr_pf_03_lists_the_keys_of_the_plans_of_the_profile', () => {
		const [group] = variableGroups('agent.label.', source);
		expect(group.id).toBe('labels');
		expect(group.options.map((option) => option.expression)).toEqual([
			'agent.label.env',
			'agent.label.role'
		]);
		expect(group.options[0].description).toBe('The host label env, for example prod');
	});

	it('fr_pf_03_offers_a_typed_key_that_no_host_sets', () => {
		const options = flat('agent.label.pg');
		expect(options.map((option) => option.expression)).toEqual(['agent.label.pg']);
		expect(options[0].description).toContain('The label pg.');
		// A key of a host is listed once.
		expect(flat('agent.label.env').map((option) => option.expression)).toEqual(['agent.label.env']);
	});

	it('fr_pf_03_reads_a_key_from_a_label_with_equals_only', () => {
		expect(hostLabels([{ labels: ['env=prod', 'role=db', 'prod'] }])).toEqual([
			{ key: 'env', value: 'prod' },
			{ key: 'role', value: 'db' }
		]);
	});
});

describe('a date format', () => {
	it('fr_pf_03_writes_the_directives_of_template_rs', () => {
		expect(dateExample('%Y-%m-%d', NOW)).toBe('2026-10-19');
		expect(dateExample('%Y%m%d', NOW)).toBe('20261019');
		expect(dateExample('%H:%M:%S', NOW)).toBe('09:42:00');
		expect(dateExample('backup-%%-%Y', NOW)).toBe('backup-%-2026');
	});

	it('fr_pf_03_refuses_an_unknown_or_unfinished_directive', () => {
		expect(dateExample('%Q', NOW)).toBeNull();
		expect(dateExample('%Y-%', NOW)).toBeNull();
	});
});

describe('the summary of the list', () => {
	it('fr_pf_03_counts_the_variables', () => {
		expect(variableCount(1)).toBe('1 variable');
		expect(variableCount(9)).toBe('9 variables');
		expect(flat('')).toHaveLength(10);
	});
});

describe('a secret in a field', () => {
	it('ac_pf_09_finds_a_secret_that_the_text_references', () => {
		expect(secretReference('/srv/{{param.pg_password}}', ['pg_password'])).toBe('pg_password');
		expect(secretReference('/srv/{{ param.pg_password }}', ['pg_password'])).toBe('pg_password');
		expect(secretReference('/srv/{{param.pg_dir}}', ['pg_password'])).toBeNull();
		expect(secretReference('{{param.pg_password', ['pg_password'])).toBeNull();
	});
});
