/*
 * The Jobs screen lists the jobs of the organization and cancels one (FR-SC-09).
 *
 * The test replaces the session store, the toast store and the service, so it renders the screen
 * alone. Only an Admin and an Operator see the cancel action; a Viewer reads.
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '#lib/api/index.ts';
import Jobs from './+page.svelte';

const mocks = vi.hoisted(() => ({
	listJobs: vi.fn(),
	cancelJob: vi.fn(),
	listAgents: vi.fn(),
	pushToast: vi.fn()
}));

const session = vi.hoisted(() => ({ role: 'operator' as string }));

const route = vi.hoisted(() => ({ url: new URL('http://localhost/jobs') }));

vi.mock('$app/state', () => ({ page: route }));
vi.mock('$app/navigation', () => ({
	goto: async (address: string) => {
		route.url = new URL(address, 'http://localhost');
	}
}));
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
			cancelJob: mocks.cancelJob,
			listAgents: mocks.listAgents,
			listRepositories: async () => [],
			listPlans: async () => []
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
		mocks.listJobs.mockResolvedValue({ items: [], next_cursor: null, total: 0 });
		mocks.listAgents.mockResolvedValue([{ id: 'a_1', hostname: 'win-fs01' }]);
	});

	it('lists a job with its status, plan, host and repository', async () => {
		mocks.listJobs.mockResolvedValue({ items: [job()], next_cursor: null, total: 1 });
		render(Jobs);

		await waitFor(() => expect(screen.getByText('nightly-web')).toBeInTheDocument());
		// The status filter lists the same words, so the query reads the table only.
		expect(within(screen.getByRole('table')).getByText('Running')).toBeInTheDocument();
		expect(screen.getByText('win-fs01')).toBeInTheDocument();
		expect(screen.getByText('primary-s3')).toBeInTheDocument();
	});

	it('reads only the jobs of the host that the link names', async () => {
		route.url = new URL('http://localhost/jobs?agent_id=a_1');
		mocks.listJobs.mockResolvedValue({ items: [job()], next_cursor: null, total: 1 });
		render(Jobs);

		await waitFor(() =>
			expect(mocks.listJobs).toHaveBeenCalledWith(expect.objectContaining({ agent_id: 'a_1' }))
		);
		const chips = await screen.findByRole('list', { name: 'Applied filters' });
		expect(chips).toHaveTextContent('Host: win-fs01');
		expect(screen.getByRole('button', { name: 'Reset' })).toBeInTheDocument();
	});

	it('shows a dash for an empty start and duration and keeps the filter in the toolbar', async () => {
		mocks.listJobs.mockResolvedValue({
			items: [job({ status: 'queued', started_at: null })],
			next_cursor: null,
			total: 1
		});
		render(Jobs);

		const table = within(await screen.findByRole('table'));
		expect((await table.findAllByText('—')).length).toBe(2);
		expect(screen.queryByText('None')).not.toBeInTheDocument();
		const filters = screen.getByRole('group', { name: 'Job filters' });
		expect(filters.closest('.table-wrap')).not.toBeNull();
	});

	it('shows a cancelled job apart from a failed one, with the pill only', async () => {
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
			next_cursor: null,
			total: 2
		});
		render(Jobs);

		const table = within(await screen.findByRole('table'));
		expect(await table.findByText('Cancelled')).toBeInTheDocument();
		expect(table.getByText('Gave up')).toBeInTheDocument();
		// The reason is on the job page (design/style/ui-principles.md, 2.5).
		expect(
			screen.queryByText('The repository stayed locked after every retry.')
		).not.toBeInTheDocument();
	});

	it('shows a job with warnings and a locked job by their status only', async () => {
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
			next_cursor: null,
			total: 2
		});
		render(Jobs);

		const table = within(await screen.findByRole('table'));
		const pill = await table.findByTitle('Succeeded with warnings');
		expect(pill).toHaveClass('status--warning');
		expect(pill.closest('.status')).toHaveAttribute('title', 'Succeeded with warnings');
		expect(pill.closest('.status')).toHaveAttribute('aria-label', 'Succeeded with warnings');
		expect(table.getByText('Failed')).toBeInTheDocument();
		expect(
			screen.queryByText(
				'Another process locks the repository. Wait for it to end and run the job again.'
			)
		).not.toBeInTheDocument();
		expect(screen.queryByText('Forget selected no snapshots')).not.toBeInTheDocument();
	});

	it('keeps the status, the start time and the duration on one line', async () => {
		mocks.listJobs.mockResolvedValue({ items: [job()], next_cursor: null, total: 1 });
		render(Jobs);

		const table = await screen.findByRole('table');
		for (const name of ['Status', 'Started', 'Duration']) {
			const header = within(table).getByRole('columnheader', { name: new RegExp(name) });
			expect(header).toHaveClass('table__cell--nowrap');
		}
	});

	it('opens the job page from the row link', async () => {
		mocks.listJobs.mockResolvedValue({ items: [job()], next_cursor: null, total: 1 });
		render(Jobs);

		const link = await screen.findByRole('link', { name: 'nightly-web' });
		expect(link).toHaveAttribute('href', expect.stringMatching(/\/jobs\/j_1$/));
		expect(screen.queryByRole('link', { name: 'win-fs01' })).not.toBeInTheDocument();
	});

	it('shows the empty state when no job exists', async () => {
		render(Jobs);
		expect(await screen.findByText('No jobs yet')).toBeInTheDocument();
	});

	it('cancels a running job after the confirmation', async () => {
		const user = userEvent.setup();
		mocks.listJobs.mockResolvedValue({ items: [job()], next_cursor: null, total: 1 });
		mocks.cancelJob.mockResolvedValue(job());
		render(Jobs);

		await screen.findByText('nightly-web');
		await user.click(screen.getByRole('button', { name: 'Cancel job' }));
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

	it('fr_ui_01 reads a numbered page from the server with the filters of the address', async () => {
		const user = userEvent.setup();
		route.url = new URL('http://localhost/jobs?status=failed&kind=backup');
		mocks.listJobs.mockResolvedValue({ items: [job()], next_cursor: null, total: 60 });
		render(Jobs);

		await screen.findByText('nightly-web');
		expect(mocks.listJobs).toHaveBeenCalledWith(
			expect.objectContaining({ status: 'failed', kind: 'backup', offset: 0, limit: 25 })
		);
		expect(screen.getByText('Showing 1 to 25 of 60')).toBeInTheDocument();
		await user.click(screen.getByRole('button', { name: 'Next' }));
		await waitFor(() =>
			expect(mocks.listJobs).toHaveBeenLastCalledWith(expect.objectContaining({ offset: 25 }))
		);
	});

	it('hides the cancel action from a Viewer', async () => {
		session.role = 'viewer';
		mocks.listJobs.mockResolvedValue({ items: [job()], next_cursor: null, total: 1 });
		render(Jobs);

		await screen.findByText('nightly-web');
		expect(
			screen.queryByRole('button', { name: 'More actions for the job nightly-web' })
		).not.toBeInTheDocument();
	});

	it('fr_ui_01 keeps the newest page when an older reload answers late', async () => {
		vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
		try {
			const user = userEvent.setup();
			let answerReload: (rows: unknown[]) => void = () => {};
			const page = (rows: unknown[]) => ({
				items: rows,
				next_cursor: null,
				total: 30,
				total_capped: false
			});
			mocks.listJobs
				.mockResolvedValueOnce(page([job({ id: 'j_1', plan_name: 'first-page' })]))
				.mockReturnValueOnce(
					new Promise((resolve) => {
						answerReload = (rows) => resolve(page(rows));
					})
				)
				.mockResolvedValue(page([job({ id: 'j_2', plan_name: 'second-page' })]));
			render(Jobs);

			await screen.findByText('first-page');
			await vi.advanceTimersByTimeAsync(15_000);
			expect(mocks.listJobs).toHaveBeenCalledTimes(2);
			await user.click(screen.getByRole('button', { name: 'Next' }));
			await screen.findByText('second-page');
			answerReload([job({ id: 'j_3', plan_name: 'stale-page' })]);
			await new Promise((resolve) => setTimeout(resolve, 0));
			expect(screen.getByText('second-page')).toBeInTheDocument();
			expect(screen.queryByText('stale-page')).not.toBeInTheDocument();
		} finally {
			vi.useRealTimers();
		}
	});

	it('fr_ui_01 asks to narrow the filters for a page past the counted rows', async () => {
		route.url = new URL('http://localhost/jobs?page=500');
		mocks.listJobs.mockRejectedValue(
			new ApiError({
				status: 400,
				code: 'invalid_argument',
				title: 'Bad Request',
				field: 'offset',
				reason: 'out_of_range'
			})
		);
		render(Jobs);

		expect(
			await screen.findByText(
				'This page is too far into the list. Narrow the filters to see older records.'
			)
		).toBeInTheDocument();
	});
});
