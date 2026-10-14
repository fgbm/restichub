/*
 * The job detail screen shows the steps, the summary, the live progress and the log of one job
 * (FR-MN-02, FR-LG-01, AC-MN-04).
 *
 * The test replaces the SvelteKit modules, the session store, the toast store and the service, so
 * it renders the screen alone.
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import JobDetailPage from './+page.svelte';

const mocks = vi.hoisted(() => ({
	getJob: vi.fn(),
	getJobLogs: vi.fn(),
	cancelJob: vi.fn(),
	pushToast: vi.fn()
}));

const session = vi.hoisted(() => ({ role: 'operator' as string }));

vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));
vi.mock('$app/state', () => ({ page: { params: { id: 'j_1' } } }));
vi.mock('#lib/stores/session.svelte.ts', () => ({
	getSession: () => ({ role: session.role })
}));
vi.mock('#lib/stores/toasts.svelte.ts', () => ({ pushToast: mocks.pushToast }));
vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		service: {
			...actual.service,
			getJob: mocks.getJob,
			getJobLogs: mocks.getJobLogs,
			cancelJob: mocks.cancelJob
		}
	};
});

const BACKUP = {
	kind: 'backup',
	files_new: 128,
	files_changed: 14,
	files_unmodified: 128_260,
	dirs_new: 6,
	dirs_changed: 3,
	dirs_unmodified: 9811,
	data_blobs: 311,
	tree_blobs: 24,
	data_added: 1_503_238_553,
	data_added_packed: 1_244_100_240,
	total_files_processed: 128_402,
	total_bytes_processed: 52_917_000_000,
	total_duration_seconds: 192.4,
	snapshot_id: 'a1b2c3d4',
	dry_run: false
};

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
		status: 'succeeded',
		result_code: 'succeeded',
		result_message: null,
		created_at: '2026-10-14T10:00:00Z',
		updated_at: '2026-10-14T10:05:00Z',
		finished_at: '2026-10-14T10:05:00Z',
		started_at: '2026-10-14T10:01:00Z',
		expires_at: null,
		run_at: '2026-10-14T10:00:00Z',
		attempt: 0,
		steps: [
			{
				step_id: 'backup',
				status: 'success',
				exit_code: 0,
				started_at: '2026-10-14T10:01:00Z',
				finished_at: '2026-10-14T10:04:12Z',
				error_code: null,
				error_params: {},
				possible_stale_lock: false,
				output: BACKUP
			}
		],
		summary: BACKUP,
		progress: null,
		log: { lines: 2, truncated: false, purged: false },
		...overrides
	};
}

function line(seq: number, text: string, level = 'info') {
	return {
		seq,
		step_id: 'backup',
		stream: 'stdout',
		level,
		ts: '2026-10-14T10:01:02Z',
		text
	};
}

function logPage(items: unknown[], overrides: Record<string, unknown> = {}) {
	return { items, has_more: false, truncated: false, purged: false, ...overrides };
}

describe('the job detail screen', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		session.role = 'operator';
		mocks.getJob.mockResolvedValue(job());
		mocks.getJobLogs.mockResolvedValue(
			logPage([line(1, 'snapshot a1b2c3d4 saved'), line(2, 'Warning: a file changed', 'warn')])
		);
	});

	it('ac_mn_04_shows_the_statistics_of_the_restic_summary', async () => {
		render(JobDetailPage);

		const summary = within((await screen.findByText('Summary')).closest('section') as HTMLElement);
		expect(summary.getByText('1.4 GiB')).toBeInTheDocument();
		expect(summary.getByText('128')).toBeInTheDocument();
		expect(summary.getByText('3m 12s')).toBeInTheDocument();
		expect(summary.getByText('a1b2c3d4')).toBeInTheDocument();
		expect(summary.getByText('Data added')).toBeInTheDocument();
		expect(summary.getByText('New files')).toBeInTheDocument();
		expect(summary.getByText('14')).toBeInTheDocument();
		expect(summary.getByText('128,260')).toBeInTheDocument();
	});

	it('shows the log lines with numbers and levels', async () => {
		render(JobDetailPage);

		const log = await screen.findByRole('log', { name: 'Job log' });
		await waitFor(() =>
			expect(within(log).getByText('snapshot a1b2c3d4 saved')).toBeInTheDocument()
		);
		expect(within(log).getByText('Warning: a file changed')).toBeInTheDocument();
		expect(within(log).getByText('WARN')).toBeInTheDocument();
		expect(within(log).getByText('2')).toBeInTheDocument();
		expect(mocks.getJobLogs).toHaveBeenCalledWith('j_1', { after_seq: 0, limit: 1000 });
	});

	it('shows the progress of the running step and offers the cancel action', async () => {
		mocks.getJob.mockResolvedValue(
			job({
				status: 'running',
				result_code: null,
				finished_at: null,
				steps: [],
				summary: null,
				progress: {
					step_id: 'backup',
					bytes_done: 700_000_000,
					total_bytes: 1_400_000_000,
					files_done: 64_000,
					total_files: 128_000,
					percent_done: 0.5,
					seconds_elapsed: 90,
					seconds_remaining: 90,
					current_files: ['C:\\Data\\big.psd'],
					updated_at: '2026-10-14T10:02:30Z'
				}
			})
		);
		mocks.getJobLogs.mockResolvedValue(logPage([]));
		render(JobDetailPage);

		expect(await screen.findByRole('progressbar', { name: 'Progress' })).toHaveValue(0.5);
		expect(screen.getByText(/64,000 of 128,000 files/)).toBeInTheDocument();
		expect(screen.getByText(/Now: C:\\Data\\big.psd/)).toBeInTheDocument();
		expect(screen.getByText('Live log')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Cancel job' })).toBeInTheDocument();
	});

	it('hides the cancel action from a Viewer', async () => {
		session.role = 'viewer';
		mocks.getJob.mockResolvedValue(job({ status: 'running', result_code: null }));
		render(JobDetailPage);

		await screen.findByText('Summary');
		expect(screen.queryByRole('button', { name: 'Cancel job' })).not.toBeInTheDocument();
	});

	it('explains a failed step with the translated error code', async () => {
		mocks.getJob.mockResolvedValue(
			job({
				status: 'failed',
				result_code: 'config_error',
				summary: null,
				steps: [
					{
						step_id: 'backup',
						status: 'config_error',
						exit_code: 12,
						started_at: '2026-10-14T10:01:00Z',
						finished_at: '2026-10-14T10:01:03Z',
						error_code: 'step_config_error',
						error_params: {},
						possible_stale_lock: false,
						output: { kind: 'none' }
					}
				]
			})
		);
		render(JobDetailPage);

		expect(
			await screen.findByText('The repository password or the access keys are wrong.')
		).toBeInTheDocument();
		expect(screen.getByText(/Configuration error/)).toBeInTheDocument();
		expect(screen.getByText(/exit code 12/)).toBeInTheDocument();
	});

	it('tells that the retention deleted the log and keeps the summary', async () => {
		mocks.getJobLogs.mockResolvedValue(logPage([], { purged: true }));
		render(JobDetailPage);

		expect(
			await screen.findByText(
				'The control plane deleted this log after the retention period. The summary stays.'
			)
		).toBeInTheDocument();
		expect(screen.getByText('1.4 GiB')).toBeInTheDocument();
	});

	it('shows an empty state for a job that does not exist', async () => {
		const { ApiError } = await import('#lib/api/index.ts');
		mocks.getJob.mockRejectedValue(
			new ApiError({ status: 404, title: 'Not Found', code: 'not_found' })
		);
		render(JobDetailPage);

		expect(await screen.findByText('This job does not exist.')).toBeInTheDocument();
	});

	it('cancels the job after the confirmation', async () => {
		const user = userEvent.setup();
		mocks.getJob.mockResolvedValue(job({ status: 'running', result_code: null }));
		mocks.cancelJob.mockResolvedValue(job({ status: 'running', result_code: null }));
		render(JobDetailPage);

		await user.click(await screen.findByRole('button', { name: 'Cancel job' }));
		const dialog = await screen.findByRole('dialog', { name: 'Cancel the job?' });
		await user.click(within(dialog).getByRole('button', { name: 'Cancel job' }));

		await waitFor(() => expect(mocks.cancelJob).toHaveBeenCalledWith('j_1'));
	});
});
