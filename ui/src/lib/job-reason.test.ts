/*
 * The reason line of an incompatible job is built from typed values in the language of the
 * reader (AC-AG-09, NFR-DOC-04).
 */
import { afterEach, describe, expect, it } from 'vitest';
import { setLocale } from '#lib/i18n/index.ts';
import { jobReason } from './job-reason';

afterEach(() => setLocale('en'));

const incompatible = { result_code: 'incompatible', result_message: 'incompatible' } as const;

describe('jobReason', () => {
	it('names the option and the versions of an incompatible job', () => {
		const params = {
			option: '--ownership-by-name',
			required_version: '0.19.0',
			installed_version: '0.18.1'
		};
		expect(jobReason({ ...incompatible, result_params: params })).toBe(
			'The option --ownership-by-name needs restic 0.19.0. The agent has restic 0.18.1.'
		);
		setLocale('ru');
		expect(jobReason({ ...incompatible, result_params: params })).toBe(
			'Параметру --ownership-by-name нужен restic 0.19.0. На агенте restic 0.18.1.'
		);
	});

	it('names the minimum version when no option is the cause', () => {
		const params = { option: null, required_version: '0.18.1', installed_version: '0.1.0' };
		expect(jobReason({ ...incompatible, result_params: params })).toBe(
			'The agent has restic 0.1.0. ResticHub needs restic 0.18.1 or later.'
		);
	});

	it('translates the code when the values are missing', () => {
		expect(jobReason(incompatible)).not.toBe('incompatible');
	});

	it('returns null for a job without a message', () => {
		expect(jobReason({ result_code: 'succeeded' })).toBeNull();
	});
});
