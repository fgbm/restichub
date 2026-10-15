/*
 * The interface builds the sentence of an alert from its typed values, in the language of the
 * reader (FR-MN-03, ADR-0036).
 */
import { afterEach, describe, expect, it } from 'vitest';
import { alertCondition } from './alert-text';
import { setLocale } from './i18n';
import type { Alert } from './api/types';

function alert(overrides: Partial<Alert>): Alert {
	return {
		id: 'al_1',
		kind: 'job_failed',
		status: 'open',
		agent_id: null,
		agent_hostname: null,
		plan_id: null,
		plan_name: null,
		repository_id: null,
		repository_name: null,
		job_id: null,
		params: {},
		error_text: null,
		url: null,
		opened_at: '2026-10-15T08:00:00Z',
		resolved_at: null,
		...overrides
	};
}

describe('alertCondition', () => {
	afterEach(() => setLocale('en'));

	it('builds an English sentence from the values', () => {
		setLocale('en');
		expect(
			alertCondition(
				alert({
					kind: 'agent_offline',
					params: { elapsed_seconds: 48 * 3600, threshold_minutes: 30 }
				})
			)
		).toBe('The agent was not seen for 48 h (threshold 30 min).');
		expect(
			alertCondition(alert({ params: { job_kind: 'backup', error_code: 'config_error' } }))
		).toBe('The backup job failed (config_error).');
	});

	it('builds a Russian sentence from the same values', () => {
		setLocale('ru');
		expect(
			alertCondition(
				alert({
					kind: 'agent_offline',
					params: { elapsed_seconds: 45 * 60, threshold_minutes: 30 }
				})
			)
		).toBe('Агент не выходил на связь 45 мин (порог 30 мин).');
	});

	it('tells a plan that never succeeded apart from one that did', () => {
		setLocale('en');
		const never = alertCondition(
			alert({
				kind: 'rpo_violated',
				params: { elapsed_seconds: 3600 * 40, rpo_seconds: 3600 * 26 }
			})
		);
		expect(never).toBe('No successful backup since the plan was created (objective 26 h).');
	});
});
