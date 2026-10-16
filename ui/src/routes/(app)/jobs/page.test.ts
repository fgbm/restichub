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

const stream = vi.hoisted(() => ({ listeners: [] as ((event: { type: string }) => void)[] }));

vi.mock('#lib/stores/events.svelte.ts', () => ({
	isStreamConnected: () => false,
	onEvent: (listener: (event: { type: string }) => void) => {
		stream.listeners.push(listener);
		return () => {
			stream.listeners = stream.listeners.filter((item) => item !== listener);
		};
	}
}));

const rendered = vi.hoisted(() => ({ meta: [] as string[] }));

vi.mock('#lib/job-item.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/job-item.ts')>();
	return {
		...actual,
		jobMeta: (...args: Parameters<typeof actual.jobMeta>) => {
			rendered.meta.push(args[0].id);
			return actual.jobMeta(...args);
		}
	};
});

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
		expect(screen.getByText('Running', { selector: '.status' })).toBeInTheDocument();
		expect(screen.getByText('win-fs01')).toBeInTheDocument();
		expect(screen.getByText('primary-s3')).toBeInTheDocument();
		// The list opens on the jobs of the last 7 days.
		expect(mocks.listJobs).toHaveBeenCalledWith(
			expect.objectContaining({ from: expect.any(String), sort: 'started_at', order: 'desc' })
		);
		expect(route.url.searchParams.get('period')).toBe('7d');
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

	it('shows the duration in the number column and a dash for a job that never started', async () => {
		mocks.listJobs.mockResolvedValue({
			items: [
				job({ id: 'j_1', status: 'succeeded', finished_at: '2026-10-14T10:05:12Z' }),
				job({ id: 'j_2', plan_name: 'gone', status: 'dead_letter', started_at: null }),
				job({ id: 'j_3', plan_name: 'waiting', status: 'queued', started_at: null })
			],
			next_cursor: null,
			total: 3
		});
		const { container } = render(Jobs);

		await screen.findByText('waiting');
		const asides = [...container.querySelectorAll('.item__aside')].map((node) => node.textContent);
		expect(asides).toEqual(['4 min 12 s', '—', '']);
		expect(screen.queryByText('None')).not.toBeInTheDocument();
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

		expect(await screen.findByText('Cancelled', { selector: '.status' })).toBeInTheDocument();
		expect(screen.getByText('Gave up', { selector: '.status' })).toBeInTheDocument();
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

		const pill = await screen.findByText('Succeeded', { selector: '.status' });
		expect(pill).toHaveClass('status--warning');
		expect(pill).toHaveAttribute('title', 'Succeeded with warnings');
		expect(pill).toHaveAttribute('aria-label', 'Succeeded with warnings');
		expect(screen.getByText('Failed', { selector: '.status' })).toBeInTheDocument();
		expect(
			screen.queryByText(
				'Another process locks the repository. Wait for it to end and run the job again.'
			)
		).not.toBeInTheDocument();
		expect(screen.queryByText('Forget selected no snapshots')).not.toBeInTheDocument();
	});

	it('shows a successful job without a pill and a grey icon', async () => {
		mocks.listJobs.mockResolvedValue({
			items: [job({ status: 'succeeded', finished_at: '2026-10-14T10:02:00Z' })],
			next_cursor: null,
			total: 1
		});
		const { container } = render(Jobs);

		await screen.findByText('nightly-web');
		expect(container.querySelector('.status')).toBeNull();
		expect(container.querySelector('.item__lead')?.className).not.toMatch(/tone--/);
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
		route.url = new URL('http://localhost/jobs?group=failed&kind=backup');
		mocks.listJobs.mockResolvedValue({ items: [job()], next_cursor: null, total: 60 });
		render(Jobs);

		await screen.findByText('nightly-web');
		expect(mocks.listJobs).toHaveBeenCalledWith(
			expect.objectContaining({ group: 'failed', kind: 'backup', offset: 0, limit: 25 })
		);
		expect(screen.getByText('Showing 1 to 25 of 60')).toBeInTheDocument();
		await user.click(screen.getByRole('button', { name: 'Next' }));
		await waitFor(() =>
			expect(mocks.listJobs).toHaveBeenLastCalledWith(expect.objectContaining({ offset: 25 }))
		);
	});

	it('fr_ui_01 shows the server counts of the state tabs and asks for the tab', async () => {
		const user = userEvent.setup();
		const tab = (count: number, capped = false) => ({ count, capped });
		mocks.listJobs.mockResolvedValue({
			items: [job()],
			next_cursor: null,
			total: 1,
			total_capped: false,
			counts: {
				all: tab(124),
				running: tab(2),
				failed: tab(3),
				succeeded: tab(10000, true),
				succeeded_with_warnings: tab(1)
			}
		});
		render(Jobs);

		const failed = await screen.findByRole('button', { name: /^Failed/ });
		expect(failed).toHaveTextContent('3');
		expect(failed.querySelector('.count')).toHaveClass('count--danger');
		expect(screen.getByRole('button', { name: /^Running/ })).toHaveTextContent('2');
		expect(screen.getByRole('button', { name: /^All/ })).toHaveTextContent('124');
		expect(screen.getByRole('button', { name: /^Succeeded/ })).toHaveTextContent('10,000+');
		const warnings = screen.getByRole('button', { name: /^With warnings/ });
		expect(warnings).toHaveTextContent('1');
		expect(warnings.querySelector('.count')).not.toHaveClass('count--danger');

		await user.click(warnings);
		await waitFor(() =>
			expect(mocks.listJobs).toHaveBeenLastCalledWith(
				expect.objectContaining({ group: 'succeeded_with_warnings', offset: 0 })
			)
		);
	});

	it('fr_ui_01 searches the names on the server and sorts by the duration', async () => {
		const user = userEvent.setup();
		mocks.listJobs.mockResolvedValue({ items: [job()], next_cursor: null, total: 1 });
		render(Jobs);

		await screen.findByText('nightly-web');
		await user.type(screen.getByRole('searchbox'), 'db-01');
		await waitFor(() =>
			expect(mocks.listJobs).toHaveBeenLastCalledWith(expect.objectContaining({ q: 'db-01' }))
		);

		route.url = new URL('http://localhost/jobs?sort=duration');
		mocks.listJobs.mockClear();
		render(Jobs);
		await waitFor(() =>
			expect(mocks.listJobs).toHaveBeenCalledWith(
				expect.objectContaining({ sort: 'duration', order: 'desc' })
			)
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

	it('fr_ui_01 reloads the rows on each job event and the tab counts at most once in 5 seconds', async () => {
		vi.useFakeTimers();
		try {
			mocks.listJobs.mockResolvedValue({ items: [job()], next_cursor: null, total: 1 });
			render(Jobs);
			await vi.advanceTimersByTimeAsync(0);
			expect(mocks.listJobs).toHaveBeenCalledTimes(1);
			expect(mocks.listJobs).toHaveBeenLastCalledWith(expect.objectContaining({ counts: true }));
			mocks.listJobs.mockClear();

			const withCounts = () =>
				mocks.listJobs.mock.calls.filter(([query]) => query.counts === true).length;
			for (let second = 0; second < 4; second += 1) {
				for (const listener of stream.listeners) listener({ type: 'job.status' });
				await vi.advanceTimersByTimeAsync(1_000);
			}
			expect(mocks.listJobs.mock.calls.length).toBeGreaterThanOrEqual(4);
			expect(withCounts()).toBe(0);
			expect(mocks.listJobs).toHaveBeenLastCalledWith(
				expect.objectContaining({ counts: undefined })
			);
			await vi.advanceTimersByTimeAsync(1_000);
			expect(withCounts()).toBe(1);
			await vi.advanceTimersByTimeAsync(5_000);
			expect(withCounts()).toBe(1);
		} finally {
			vi.useRealTimers();
		}
	});

	it('fr_ui_02 keeps the spinner and the unchanged rows across a job event', async () => {
		vi.useFakeTimers();
		try {
			// Each read answers with new objects, as the server does.
			mocks.listJobs.mockImplementation(async () => ({
				items: [
					job(),
					job({
						id: 'j_2',
						plan_name: 'weekly-db',
						status: 'succeeded',
						result_code: 'succeeded',
						finished_at: '2026-10-14T09:30:00Z'
					})
				],
				next_cursor: null,
				total: 2
			}));
			render(Jobs);
			await vi.advanceTimersByTimeAsync(0);
			const spinner = document.querySelector('.item .spin');
			const finished = screen.getByText('weekly-db');
			expect(spinner).not.toBeNull();
			mocks.listJobs.mockClear();
			rendered.meta = [];

			for (const listener of stream.listeners) listener({ type: 'job.status' });
			await vi.advanceTimersByTimeAsync(1_000);

			expect(mocks.listJobs).toHaveBeenCalled();
			expect(document.querySelector('.item .spin')).toBe(spinner);
			expect(spinner?.isConnected).toBe(true);
			expect(screen.getByText('weekly-db')).toBe(finished);
			// Only the running row renders again: its duration counts from now.
			expect(rendered.meta).toEqual(['j_1']);
		} finally {
			vi.useRealTimers();
		}
	});

	it('fr_ui_01 opens the tab of the status that an older link names', async () => {
		route.url = new URL('http://localhost/jobs?status=failed');
		render(Jobs);

		await waitFor(() =>
			expect(mocks.listJobs).toHaveBeenLastCalledWith(expect.objectContaining({ group: 'failed' }))
		);
		expect(route.url.searchParams.get('group')).toBe('failed');
		expect(route.url.searchParams.has('status')).toBe(false);

		route.url = new URL('http://localhost/jobs?status=queued&group=running');
		mocks.listJobs.mockClear();
		render(Jobs);
		await waitFor(() => expect(mocks.listJobs).toHaveBeenCalled());
		expect(mocks.listJobs).toHaveBeenLastCalledWith(expect.objectContaining({ group: undefined }));
		expect(route.url.searchParams.has('status')).toBe(false);
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
