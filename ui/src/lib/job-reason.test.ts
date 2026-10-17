/*
 * The reason line of an incompatible job is built from typed values in the language of the
 * reader (AC-AG-09, NFR-DOC-04).
 */
import { afterEach, describe, expect, it } from 'vitest';
import { setLocale } from '#lib/i18n/index.ts';
import { jobDetailReason, jobReason, notStartedReason } from './job-reason';

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

	it('translates a typed result message and keeps a code without a translation', () => {
		expect(jobReason({ result_code: 'failed', result_message: 'interrupted' })).toBe(
			'Interrupted: the agent restarted while scripts ran'
		);
		expect(jobReason({ result_code: 'failed', result_message: 'lock_retries_exhausted' })).toBe(
			'The repository stayed locked after every retry.'
		);
		expect(jobReason({ result_code: 'failed', result_message: 'brand_new_code' })).toBe(
			'brand_new_code'
		);
	});
});

describe('the reason of a step that did not start', () => {
	const step = (params: Record<string, string>) => ({
		error_code: 'step_not_started',
		error_params: params
	});

	it('fr_ag_09_names_the_cause_that_the_agent_sent', () => {
		const job = {
			result_code: 'failed' as const,
			result_message: 'step_not_started',
			result_params: null,
			steps: [{ error_code: null, error_params: {} }, step({ reason: 'cannot protect C:\\x' })]
		};
		expect(notStartedReason(job.steps[1])).toBe('cannot protect C:\\x');
		expect(jobDetailReason(job)).toBe('restic did not start. Agent message: cannot protect C:\\x');
		setLocale('ru');
		expect(jobDetailReason(job)).toBe(
			'restic не запустился. Сообщение агента: cannot protect C:\\x'
		);
	});

	it('fr_ag_09_keeps_the_general_text_without_a_cause', () => {
		expect(notStartedReason(step({}))).toBeNull();
		expect(
			notStartedReason({ error_code: 'step_failed', error_params: { reason: 'x' } })
		).toBeNull();
		const job = {
			result_code: 'failed' as const,
			result_message: 'step_not_started',
			result_params: null,
			steps: [step({ reason: '  ' })]
		};
		expect(jobDetailReason(job)).toBe('restic did not start. Read the job log.');
	});
});
