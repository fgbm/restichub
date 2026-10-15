/*
 * The Alerts screen lists the open alerts, the resolved alerts and all alerts, and pages with a
 * cursor (FR-MN-03, AC-MN-02).
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Alerts from './+page.svelte';

const mocks = vi.hoisted(() => ({ listAlerts: vi.fn(), pushToast: vi.fn() }));

vi.mock('#lib/stores/toasts.svelte.ts', () => ({ pushToast: mocks.pushToast }));

vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return { ...actual, service: { ...actual.service, listAlerts: mocks.listAlerts } };
});

const PAST = '2026-10-15T08:00:00Z';

function alert(overrides: Record<string, unknown> = {}) {
	return {
		id: 'al_1',
		kind: 'job_failed',
		status: 'open',
		agent_id: 'a_1',
		agent_hostname: 'win-fs01',
		plan_id: 'pl_1',
		plan_name: 'nightly-web',
		repository_id: 'r_1',
		repository_name: 'primary-s3',
		job_id: 'j_9',
		params: { job_kind: 'backup', error_code: 'config_error' },
		error_text: 'compile_failed',
		url: 'https://backup.example.com/jobs/j_9',
		opened_at: PAST,
		resolved_at: null,
		...overrides
	};
}

describe('the Alerts screen', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it('lists an open alert with its subject, its text and the link to the job', async () => {
		mocks.listAlerts.mockResolvedValue({ items: [alert()], next_cursor: null });
		render(Alerts);

		await waitFor(() => expect(screen.getByText('nightly-web')).toBeInTheDocument());
		expect(mocks.listAlerts).toHaveBeenCalledWith({ status: 'open', limit: 50 });
		expect(screen.getByText('Job failed')).toBeInTheDocument();
		expect(screen.getByText('The backup job failed (config_error).')).toBeInTheDocument();
		expect(screen.getByText('compile_failed')).toBeInTheDocument();
		expect(screen.getByRole('link', { name: /Open the job/ })).toHaveAttribute('href', '/jobs/j_9');
		expect(screen.getByText('win-fs01 · primary-s3')).toBeInTheDocument();
	});

	it('shows the empty state of the open list', async () => {
		mocks.listAlerts.mockResolvedValue({ items: [], next_cursor: null });
		render(Alerts);

		expect(await screen.findByText('No open alerts')).toBeInTheDocument();
	});

	it('asks for the resolved alerts when the filter changes', async () => {
		const user = userEvent.setup();
		mocks.listAlerts
			.mockResolvedValueOnce({ items: [alert()], next_cursor: null })
			.mockResolvedValueOnce({
				items: [
					alert({
						id: 'al_0',
						kind: 'rpo_violated',
						status: 'resolved',
						job_id: null,
						params: { elapsed_seconds: 31 * 3600, rpo_seconds: 26 * 3600 },
						error_text: null,
						resolved_at: '2026-10-15T09:00:00Z'
					})
				],
				next_cursor: null
			});
		render(Alerts);
		await screen.findByText('nightly-web');

		await user.selectOptions(screen.getByLabelText('Show alerts'), 'resolved');

		await waitFor(() =>
			expect(mocks.listAlerts).toHaveBeenLastCalledWith({ status: 'resolved', limit: 50 })
		);
		const row = (await screen.findByText('Backup overdue')).closest('tr') as HTMLElement;
		expect(within(row).getByText('Resolved')).toBeInTheDocument();
	});

	it('loads the next page with the cursor', async () => {
		const user = userEvent.setup();
		mocks.listAlerts
			.mockResolvedValueOnce({ items: [alert()], next_cursor: 'c1' })
			.mockResolvedValueOnce({
				items: [alert({ id: 'al_2', plan_name: 'weekly-db', job_id: null })],
				next_cursor: null
			});
		render(Alerts);
		await screen.findByText('nightly-web');

		await user.click(screen.getByRole('button', { name: 'Load more' }));

		await waitFor(() => expect(screen.getByText('weekly-db')).toBeInTheDocument());
		expect(mocks.listAlerts).toHaveBeenLastCalledWith({
			status: 'open',
			limit: 50,
			cursor: 'c1'
		});
		expect(screen.getByText('nightly-web')).toBeInTheDocument();
	});
});
