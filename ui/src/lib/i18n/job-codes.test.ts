/*
 * Every job status, result code, kind and step status of the API has a translation, so the screens
 * never show a raw code (NFR-DOC-04). A new value in the API schema fails the type check here
 * until the list below holds it.
 */
import { describe, expect, it } from 'vitest';
import type { components } from '../api/schema';
import { en } from './en';
import { ru } from './ru';

type Schemas = components['schemas'];

const JOB_STATUS: Record<Schemas['JobStatus'], true> = {
	queued: true,
	sent: true,
	running: true,
	succeeded: true,
	failed: true,
	expired: true,
	dead_letter: true
};

const JOB_KIND: Record<Schemas['JobKind'], true> = {
	init: true,
	test_access: true,
	backup: true,
	check: true,
	forget: true,
	prune: true,
	restore: true,
	unlock: true,
	unlock_all: true,
	snapshots: true,
	ls: true,
	other: true
};

const STEP_STATUS: Record<Schemas['StepStatus'], true> = {
	success: true,
	warning: true,
	retry: true,
	config_error: true,
	cancelled: true,
	failed: true,
	skipped: true
};

// The codes that the agent and the control plane write into `result_message`.
const RESULT_MESSAGES = [
	'interrupted',
	'step_failed',
	'wrong_password',
	'repository_missing',
	'hook_failed',
	'stalled',
	'max_duration_exceeded',
	'repository_locked',
	'agent_lost',
	'expired',
	'cancelled',
	'succeeded_with_warnings'
];

// The result codes that the Jobs screens turn into a label of their own.
const DERIVED_STATUSES = ['cancelled', 'succeeded_with_warnings'];

describe.each([
	['en', en],
	['ru', ru]
])('the %s translations of the job codes', (_name, messages) => {
	const has = (key: string) => key in messages;

	it('cover every job status', () => {
		const keys = [...Object.keys(JOB_STATUS), ...DERIVED_STATUSES];
		expect(keys.filter((code) => !has(`jobs.status.${code}`))).toEqual([]);
	});

	it('cover every job kind', () => {
		expect(Object.keys(JOB_KIND).filter((code) => !has(`jobs.kind.${code}`))).toEqual([]);
	});

	it('cover every step status', () => {
		expect(Object.keys(STEP_STATUS).filter((code) => !has(`jobDetail.stepStatus.${code}`))).toEqual(
			[]
		);
	});

	it('cover every result message as a reason and as a short code', () => {
		expect(RESULT_MESSAGES.filter((code) => !has(`jobs.reason.${code}`))).toEqual([]);
		expect(
			RESULT_MESSAGES.filter((code) => code !== 'succeeded_with_warnings').filter(
				(code) => !has(`jobs.errorCode.${code}`)
			)
		).toEqual([]);
	});
});
