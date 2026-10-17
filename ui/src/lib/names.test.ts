import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { MAX_RECORD_NAME, defaultPlanName, nameProblem } from './names.ts';

const root = join(process.cwd(), '..');
const vectors = JSON.parse(
	readFileSync(join(root, 'crates/restichub-cp/tests/fixtures/name-rules.json'), 'utf8')
) as Record<'valid' | 'empty' | 'invalid_character' | 'too_long', string[]>;
const serverSource = readFileSync(join(root, 'crates/restichub-cp/src/validation.rs'), 'utf8');

describe('record names', () => {
	it('has the length limit of the control plane', () => {
		const match = serverSource.match(/pub const MAX_RECORD_NAME: usize = (\d+);/);
		expect(match, 'MAX_RECORD_NAME in validation.rs').not.toBeNull();
		expect(MAX_RECORD_NAME).toBe(Number(match?.[1]));
	});

	// The control plane runs the same list in its test `fr_pf_01_the_name_vectors_match_the_ui`.
	it.each(['valid', 'empty', 'invalid_character', 'too_long'] as const)(
		'gives the verdict of the control plane for the %s names',
		(kind) => {
			for (const name of vectors[kind]) {
				expect(nameProblem(name), JSON.stringify(name)).toBe(kind === 'valid' ? null : kind);
			}
		}
	);

	it('counts a character, not a UTF-16 unit', () => {
		expect(nameProblem('📦'.repeat(MAX_RECORD_NAME))).toBeNull();
		expect(nameProblem('📦'.repeat(MAX_RECORD_NAME + 1))).toBe('too_long');
	});
});

describe('default plan name', () => {
	it('joins the host and the profile as they are', () => {
		expect(defaultPlanName('Restichub-Load-Test', 'linux-logs', [])).toBe(
			'Restichub-Load-Test | linux-logs'
		);
	});

	it('is empty until the host and the profile are known', () => {
		expect(defaultPlanName('', 'linux-logs', [])).toBe('');
		expect(defaultPlanName('host', '', [])).toBe('');
	});

	it('adds a number when the name is taken, without regard to case', () => {
		const base = 'host | logs';
		expect(defaultPlanName('host', 'logs', ['HOST | LOGS'])).toBe(`${base} (2)`);
		expect(defaultPlanName('host', 'logs', [base, `${base} (2)`])).toBe(`${base} (3)`);
	});

	it('stays within the limit with the number', () => {
		const long = defaultPlanName('h'.repeat(80), 'p'.repeat(80), []);
		expect([...long].length).toBe(MAX_RECORD_NAME);
		const second = defaultPlanName('h'.repeat(80), 'p'.repeat(80), [long]);
		expect([...second].length).toBeLessThanOrEqual(MAX_RECORD_NAME);
		expect(second.endsWith(' (2)')).toBe(true);
		expect(nameProblem(second)).toBeNull();
	});
});
