/*
 * The Jobs screen lists the jobs of the organization and cancels one (FR-SC-09).
 *
 * The test replaces the session store, the toast store and the service, so it renders the screen
 * alone. Only an Admin and an Operator see the cancel action; a Viewer reads.
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Jobs from './+page.svelte';

const mocks = vi.hoisted(() => ({
	listJobs: vi.fn(),
	cancelJob: vi.fn(),
	pushToast: vi.fn()
}));

const session = vi.hoisted(() => ({ role: 'operator' as string }));

const route = vi.hoisted(() => ({ url: new URL('http://localhost/jobs') }));

vi.mock('$app/state', () => ({ page: route }));
vi.mock('#lib/stores/session.svelte.ts', () => ({
	getSession: () => ({
		user: {
			id: 'u_01',
			username: 'operator',
			display_name: 'Olga Ivanova',
			role: 'operator',
			disabled: false,
			shared: false
		},
		tenant: { id: 't_01', name: 'acme' },
		role: session.role
	})
}));

vi.mock('#lib/stores/toasts.svelte.ts', () => ({ pushToast: mocks.pushToast }));

vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		service: {
			...actual.service,
			listJobs: mocks.listJobs,
			cancelJob: mocks.cancelJob
		}
	};
});

function job(overrides: Record<string, unknown> = {}) {
	return {
		id: 'j_1',
		kind: 'backup',
		repository_id: 'r_1',
		repository_name: 'primary-s3',
		agent_id: 'a_1',
		agent_hostname: 'win-fs01',
		plan_id: 'pl_1',
		plan_name: 'nightly-web',
		status: 'running',
		result_code: null,
		result_message: null,
		created_at: '2026-10-14T10:00:00Z',
		updated_at: '2026-10-14T10:05:00Z',
		finished_at: null,
		started_at: '2026-10-14T10:01:00Z',
		expires_at: null,
		run_at: '2026-10-14T10:00:00Z',
		attempt: 0,
		...overrides
	};
}

describe('the Jobs screen', () => {
	beforeEach(() => {
		route.url = new URL('http://localhost/jobs');
		vi.clearAllMocks();
		session.role = 'operator';
		mocks.listJobs.mockResolvedValue({ items: [], next_cursor: null });
	});

	it('lists a job with its status, plan, host and repository', async () => {
		mocks.listJobs.mockResolvedValue({ items: [job()], next_cursor: null });
		render(Jobs);

		await waitFor(() => expect(screen.getByText('nightly-web')).toBeInTheDocument());
		// The status filter lists the same words, so the query reads the table only.
		expect(within(screen.getByRole('table')).getByText('Running')).toBeInTheDocument();
		expect(screen.getByText('win-fs01')).toBeInTheDocument();
		expect(screen.getByText('primary-s3')).toBeInTheDocument();
	});

	it('reads only the jobs of the host that the link names', async () => {
		route.url = new URL('http://localhost/jobs?agent_id=a_01');
		mocks.listJobs.mockResolvedValue({ items: [job()], next_cursor: null });
		render(Jobs);

		await waitFor(() =>
			expect(mocks.listJobs).toHaveBeenCalledWith(expect.objectContaining({ agent_id: 'a_01' }))
		);
		expect(await screen.findByText('Host: win-fs01')).toBeInTheDocument();
		expect(screen.getByRole('link', { name: 'Show all jobs' })).toBeInTheDocument();
	});

	it('shows a dash for an empty start and duration and keeps the filter in the toolbar', async () => {
		mocks.listJobs.mockResolvedValue({
			items: [job({ status: 'queued', started_at: null })],
			next_cursor: null
		});
		render(Jobs);

		const table = within(await screen.findByRole('table'));
		expect((await table.findAllByText('—')).length).toBe(2);
		expect(screen.queryByText('None')).not.toBeInTheDocument();
		const filter = screen.getByRole('combobox', { name: 'Status filter' });
		expect(filter.closest('.table-toolbar')).not.toBeNull();
	});

	it('shows a cancelled job apart from a failed one and translates the reason', async () => {
		mocks.listJobs.mockResolvedValue({
			items: [
				job({
					id: 'j_2',
					status: 'failed',
					result_code: 'cancelled',
					finished_at: '2026-10-14T10:03:00Z'
				}),
				job({
					id: 'j_3',
					plan_name: 'weekly-db',
					status: 'dead_letter',
					result_code: 'failed',
					result_message: 'lock_retries_exhausted',
					finished_at: '2026-10-14T10:03:00Z'
				})
			],
			next_cursor: null
		});
		render(Jobs);

		const table = within(await screen.findByRole('table'));
		expect(await table.findByText('Cancelled')).toBeInTheDocument();
		expect(table.getByText('Gave up')).toBeInTheDocument();
		expect(screen.getByText('The repository stayed locked after every retry.')).toBeInTheDocument();
	});

	it('shows a job with warnings and a locked job with their typed results', async () => {
		mocks.listJobs.mockResolvedValue({
			items: [
				job({
					id: 'j_4',
					status: 'succeeded',
					result_code: 'succeeded_with_warnings',
					result_message: 'forget_selected_nothing',
					finished_at: '2026-10-14T10:03:00Z'
				}),
				job({
					id: 'j_5',
					status: 'failed',
					result_code: 'repository_locked',
					finished_at: '2026-10-14T10:03:00Z'
				})
			],
			next_cursor: null
		});
		render(Jobs);

		const table = within(await screen.findByRole('table'));
		expect(await table.findByText('Succeeded with warnings')).toBeInTheDocument();
		expect(
			screen.getByText(
				'Another process locks the repository. Wait for it to end and run the job again.'
			)
		).toBeInTheDocument();
		expect(screen.getByText('Forget selected no snapshots')).toBeInTheDocument();
	});

	it('keeps the status, the start time and the duration on one line', async () => {
		mocks.listJobs.mockResolvedValue({ items: [job()], next_cursor: null });
		render(Jobs);

		const table = await screen.findByRole('table');
		for (const name of ['Status', 'Started', 'Duration']) {
			const header = within(table).getByRole('columnheader', { name: new RegExp(name) });
			expect(header).toHaveClass('table__cell--nowrap');
		}
	});

	it('translates the interrupted result of a job', async () => {
		mocks.listJobs.mockResolvedValue({
			items: [
				job({
					id: 'j_6',
					status: 'failed',
					result_code: 'failed',
					result_message: 'interrupted',
					finished_at: '2026-10-14T10:03:00Z'
				})
			],
			next_cursor: null
		});
		render(Jobs);

		expect(
			await screen.findByText('Interrupted: the agent restarted while scripts ran')
		).toBeInTheDocument();
		expect(screen.queryByText('interrupted')).not.toBeInTheDocument();
	});

	it('shows the raw code of a result that has no translation', async () => {
		mocks.listJobs.mockResolvedValue({
			items: [
				job({
					id: 'j_7',
					status: 'failed',
					result_code: 'failed',
					result_message: 'brand_new_code',
					finished_at: '2026-10-14T10:03:00Z'
				})
			],
			next_cursor: null
		});
		render(Jobs);

		expect(await screen.findByText('brand_new_code')).toBeInTheDocument();
	});

	it('shows the empty state when no job exists', async () => {
		render(Jobs);
		expect(await screen.findByText('No jobs yet')).toBeInTheDocument();
	});

	it('cancels a running job after the confirmation', async () => {
		const user = userEvent.setup();
		mocks.listJobs.mockResolvedValue({ items: [job()], next_cursor: null });
		mocks.cancelJob.mockResolvedValue(job());
		render(Jobs);

		await screen.findByText('nightly-web');
		await user.click(screen.getByRole('button', { name: 'More actions for the job nightly-web' }));
		// The menu content is hidden in jsdom, so the text query finds the item.
		await user.click(await screen.findByText('Cancel job'));
		const dialog = await screen.findByRole('dialog');
		expect(mocks.cancelJob).not.toHaveBeenCalled();
		await user.click(within(dialog).getByRole('button', { name: 'Cancel job' }));

		await waitFor(() => expect(mocks.cancelJob).toHaveBeenCalledWith('j_1'));
		await waitFor(() =>
			expect(mocks.pushToast).toHaveBeenCalledWith(
				'success',
				'The cancel request is sent to the host.'
			)
		);
	});

	it('loads the next page with the cursor', async () => {
		const user = userEvent.setup();
		mocks.listJobs
			.mockResolvedValueOnce({ items: [job()], next_cursor: 'c1' })
			.mockResolvedValueOnce({
				items: [job({ id: 'j_9', plan_name: 'older-plan' })],
				next_cursor: null
			});
		render(Jobs);

		await screen.findByText('nightly-web');
		await user.click(screen.getByRole('button', { name: 'Load more' }));
		await screen.findByText('older-plan');
		expect(mocks.listJobs).toHaveBeenLastCalledWith(expect.objectContaining({ cursor: 'c1' }));
	});

	it('hides the cancel action from a Viewer', async () => {
		session.role = 'viewer';
		mocks.listJobs.mockResolvedValue({ items: [job()], next_cursor: null });
		render(Jobs);

		await screen.findByText('nightly-web');
		expect(
			screen.queryByRole('button', { name: 'More actions for the job nightly-web' })
		).not.toBeInTheDocument();
	});
});
