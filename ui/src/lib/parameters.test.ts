/*
 * The profile parameters in the screens: the rule of a name, the uses of a parameter in a draft,
 * the order of the value resolution and the refusal that names the plans (FR-PF-09 to FR-PF-12,
 * AC-PF-07, AC-PF-08).
 */
import { describe, expect, it } from 'vitest';
import type { ApiProblem } from '#lib/api/index.ts';
import {
	kindLabel,
	missingValue,
	parameterChip,
	parameterNameError,
	plansRefusal,
	resolveValue,
	usageOf,
	usageText
} from './parameters';

function definition(overrides = {}) {
	return {
		name: 'pg_dir',
		kind: 'text' as const,
		required: false,
		default_value: null,
		description: null,
		hidden: false,
		...overrides
	};
}

function hostRow(overrides = {}) {
	return {
		name: 'pg_dir',
		kind: 'text' as const,
		required: false,
		default_value: null,
		description: null,
		needed_by: ['pg-dump'],
		value: null,
		unused: false,
		hidden: false,
		...overrides
	};
}

describe('the name of a parameter', () => {
	it('fr_pf_09_checks_the_name_against_the_rule_of_the_server', () => {
		expect(parameterNameError('pg_host')).toBeNull();
		expect(parameterNameError('a1_b')).toBeNull();
		expect(parameterNameError('Pg')).not.toBeNull();
		expect(parameterNameError('1pg')).not.toBeNull();
		expect(parameterNameError('pg-host')).not.toBeNull();
		expect(parameterNameError('p'.repeat(65))).not.toBeNull();
		// An empty row is a row that the person did not fill yet, so it has no error.
		expect(parameterNameError('')).toBeNull();
	});

	it('fr_pf_09_shows_the_kind_and_the_count_of_a_profile', () => {
		expect(kindLabel('text')).toBe('Text');
		expect(kindLabel('secret')).toBe('Secret');
		expect(parameterChip([])).toBeNull();
		expect(parameterChip([definition()])).toBe('1 parameter');
		expect(parameterChip([definition(), definition({ name: 'a' })])).toBe('2 parameters');
	});
});

describe('the uses of a parameter', () => {
	it('fr_pf_10_counts_the_paths_tags_and_script_variables', () => {
		const usage = usageOf('pg_dir', {
			paths: ['{{param.pg_dir}}/dump', '/etc'],
			excludes: ['{{param.pg_dir}}/*.tmp'],
			excludeFiles: [],
			tags: ['{{param.pg_dir}}'],
			scripts: [
				{
					key: 1,
					name: 'dump',
					interpreter: 'bash',
					phase: 'before',
					body: '',
					timeoutSeconds: '',
					failurePolicy: 'abort_backup',
					templateId: null,
					open: false,
					env: [{ name: 'DUMP_DIR', kind: 'parameter', value: 'pg_dir' }]
				}
			]
		});
		expect(usage).toEqual({ paths: 1, excludes: 1, excludeFiles: 0, tags: 1, scripts: 1 });
		expect(usageText(usage)).toBe('1 path · 1 exclusion · 1 tag · 1 script');
		expect(usageText({ paths: 0, excludes: 0, excludeFiles: 0, tags: 0, scripts: 0 })).toBe('—');
	});
});

describe('the resolution of a value', () => {
	const secret = definition({ name: 'pg_password', kind: 'secret', required: true });

	it('ac_pf_08_resolves_the_plan_then_the_host_then_the_default', () => {
		const withDefault = definition({ default_value: '/var/backups/pg' });
		expect(
			resolveValue(
				withDefault,
				[
					{
						name: 'pg_dir',
						kind: 'text',
						value: '/plan',
						secret_id: null,
						secret_name: null,
						hidden: false
					}
				],
				[hostRow({ value: { kind: 'text', value: '/host', secret_id: null, secret_name: null } })]
			)
		).toEqual({ source: 'plan', text: '/plan', secretName: null, hidden: false });

		expect(
			resolveValue(
				withDefault,
				[],
				[hostRow({ value: { kind: 'text', value: '/host', secret_id: null, secret_name: null } })]
			)
		).toEqual({ source: 'host', text: '/host', secretName: null, hidden: false });

		expect(resolveValue(withDefault, [], [])).toEqual({
			source: 'default',
			text: '/var/backups/pg',
			secretName: null,
			hidden: false
		});
	});

	it('ac_pf_08_reads_a_secret_as_its_name_and_never_as_a_text', () => {
		expect(
			resolveValue(
				secret,
				[],
				[
					hostRow({
						name: 'pg_password',
						kind: 'secret',
						value: {
							kind: 'secret',
							value: null,
							secret_id: 's_1',
							secret_name: 'db.password'
						}
					})
				]
			)
		).toEqual({ source: 'host', text: '', secretName: 'db.password', hidden: false });
	});

	it('ac_pf_06_names_a_required_parameter_without_a_value', () => {
		expect(missingValue(definition({ required: true }), [], [])).toBe(true);
		expect(missingValue(definition({ required: true, default_value: 'x' }), [], [])).toBe(false);
		expect(missingValue(definition({ required: false }), [], [])).toBe(false);
		expect(
			missingValue(
				definition({ required: true }),
				[],
				[hostRow({ value: { kind: 'text', value: 'x', secret_id: null, secret_name: null } })]
			)
		).toBe(false);
	});
});

describe('the refusal of a change', () => {
	it('ac_pf_07_reads_the_plans_and_the_names_of_the_refusal', () => {
		const problem: ApiProblem = {
			status: 400,
			code: 'parameter_missing_in_plans',
			title: 'Bad Request',
			parameters: ['pg_password'],
			blockers: {
				items: [
					{ kind: 'plan', id: 'pl_1', name: 'pg-dump-daily' },
					{ kind: 'plan', id: 'pl_2', name: 'pg-dump-fast' }
				],
				total: 3
			}
		};
		const refused = plansRefusal(problem);
		expect(refused?.names).toEqual(['pg_password']);
		expect(refused?.plans.links).toEqual([
			{ label: 'pg-dump-daily', kind: 'Plan', href: '/plans/pl_1' },
			{ label: 'pg-dump-fast', kind: 'Plan', href: '/plans/pl_2' }
		]);
		expect(refused?.plans.more).toBe(1);
		expect(
			plansRefusal({ status: 400, code: 'parameter_missing', title: 'Bad Request' })
		).toBeNull();
	});
});
