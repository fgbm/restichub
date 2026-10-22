/*
 * The job page names the reason of a job that an interruption cancelled, and keeps a plain cancel
 * without a reason line (FR-MN-03, AC-MN-10). The test replaces the session, the toast store and
 * the service, so it renders the page alone.
 */
import { render, screen } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import JobPage from './JobPage.svelte';

const mocks = vi.hoisted(() => ({
	getJob: vi.fn(),
	getJobLogs: vi.fn()
}));

vi.mock('$app/paths', () => ({
	resolve: (path: string, params?: Record<string, string>) =>
		params ? path.replace('[id]', params.id) : path
}));
vi.mock('#lib/stores/session.svelte.ts', () => ({
	getSession: () => ({
		user: { id: 'u_01', username: 'admin', display_name: 'Anna', role: 'admin' },
		tenant: { id: 't_01', name: 'acme' },
		role: 'admin'
	})
}));
vi.mock('#lib/stores/toasts.svelte.ts', () => ({ pushToast: vi.fn() }));
vi.mock('#lib/stores/events.svelte.ts', () => ({
	isStreamConnected: () => false,
	onEvent: () => () => {}
}));
vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		service: { ...actual.service, getJob: mocks.getJob, getJobLogs: mocks.getJobLogs }
	};
});

function job(overrides: Record<string, unknown> = {}) {
	return {
		id: 'j_01',
		kind: 'backup',
		status: 'failed',
		result_code: 'cancelled',
		result_message: null,
		result_params: null,
		agent_id: 'a_01',
		agent_hostname: 'srv-db-01',
		plan_id: 'p_01',
		plan_name: 'nightly',
		repository_id: 'r_01',
		repository_name: 'main',
		attempt: 1,
		run_at: '2026-10-22T02:00:00Z',
		created_at: '2026-10-22T02:00:00Z',
		updated_at: '2026-10-22T02:05:00Z',
		started_at: '2026-10-22T02:00:05Z',
		finished_at: '2026-10-22T02:05:00Z',
		expires_at: null,
		limits: {},
		log: { lines: 0, purged: false, truncated: false },
		planned: [],
		planned_steps: [],
		progress: null,
		restore: null,
		signing_key_id: null,
		steps: [],
		summary: null,
		trigger: 'manual',
		...overrides
	};
}

describe('the job page of a cancelled job', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mocks.getJobLogs.mockResolvedValue({
			items: [],
			has_more: false,
			purged: false,
			truncated: false
		});
	});

	it('ac_mn_10_names_the_interruption_that_cancelled_the_job', async () => {
		mocks.getJob.mockResolvedValue(job({ result_message: 'interrupted' }));
		render(JobPage, { jobId: 'j_01' });

		expect(
			await screen.findByText('Interrupted: the agent restarted while scripts ran')
		).toBeVisible();
	});

	it('ac_mn_10_shows_no_reason_for_a_plain_cancel', async () => {
		for (const message of [null, 'cancelled']) {
			mocks.getJob.mockResolvedValue(job({ result_message: message }));
			const { unmount } = render(JobPage, { jobId: 'j_01' });

			expect(await screen.findByRole('heading', { level: 1, name: 'j_01' })).toBeVisible();
			expect(screen.queryByText('The job was cancelled.')).toBeNull();
			unmount();
		}
	});

	it('ac_mn_10_keeps_the_reason_of_a_failed_job', async () => {
		mocks.getJob.mockResolvedValue(
			job({ result_code: 'failed', result_message: 'agent_lost', kind: 'prune' })
		);
		render(JobPage, { jobId: 'j_01' });

		expect(await screen.findByText(/The host went offline during the job/)).toBeVisible();
	});
});
