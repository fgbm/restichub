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
	runPlan: vi.fn(),
	goto: vi.fn(),
	pushToast: vi.fn()
}));

const session = vi.hoisted(() => ({ role: 'operator' as string }));

vi.mock('$app/paths', () => ({
	resolve: (path: string, params?: Record<string, string>) =>
		params ? path.replace('[id]', params.id) : path
}));
vi.mock('$app/navigation', () => ({ goto: mocks.goto }));
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
			cancelJob: mocks.cancelJob,
			runPlan: mocks.runPlan
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
		trigger: 'schedule',
		planned_steps: ['backup'],
		planned: [{ step_id: 'backup', kind: 'backup', name: '', phase: null }],
		steps: [
			{
				step_id: 'backup',
				kind: 'backup',
				name: '',
				phase: null,
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
		limits: { max_duration_seconds: null, stall_timeout_seconds: 3600 },
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

	it('hides_the_summary_panel_until_a_step_summary_exists', async () => {
		mocks.getJob.mockResolvedValue(job({ status: 'running', result_code: null, summary: null }));
		render(JobDetailPage);

		await screen.findByText('About this job');
		expect(screen.queryByText('Summary')).not.toBeInTheDocument();
		expect(screen.queryByText('The summary appears when the first step finishes.')).toBeNull();
	});

	it('shows_the_overall_progress_bar_only_with_more_than_one_step', async () => {
		mocks.getJob.mockResolvedValue(job({ planned_steps: ['backup'] }));
		const { unmount } = render(JobDetailPage);
		await screen.findByText('About this job');
		expect(screen.queryByRole('progressbar', { name: 'Steps' })).not.toBeInTheDocument();
		unmount();

		mocks.getJob.mockResolvedValue(job({ planned_steps: ['backup', 'forget'] }));
		render(JobDetailPage);
		expect(await screen.findByRole('progressbar', { name: 'Steps' })).toBeInTheDocument();
	});

	it('truncates_the_job_id_in_the_about_panel_with_a_tooltip_and_a_copy_button', async () => {
		const user = userEvent.setup();
		const writeText = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue();
		render(JobDetailPage);

		await screen.findByText('About this job');
		const id = screen.getAllByText('j_1').find((node) => node.getAttribute('title') === 'j_1');
		expect(id).toBeDefined();
		await user.click(screen.getByRole('button', { name: 'Copy the job ID' }));
		expect(writeText).toHaveBeenCalledWith('j_1');
	});

	it('fr_mn_02_shows_the_signing_key_with_a_copy_button', async () => {
		const user = userEvent.setup();
		const writeText = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue();
		mocks.getJob.mockResolvedValue(job({ signing_key_id: '3f9c2a71-5b4e-4d08-9a6c-1e7d0b8f42ac' }));
		render(JobDetailPage);

		const key = await screen.findByText('3f9c2a71');
		expect(key).toHaveClass('mono');
		expect(key).toHaveAttribute('title', '3f9c2a71-5b4e-4d08-9a6c-1e7d0b8f42ac');
		expect(screen.getByText('Signing key')).toBeInTheDocument();
		await user.click(screen.getByRole('button', { name: 'Copy the signing key ID' }));
		expect(writeText).toHaveBeenCalledWith('3f9c2a71-5b4e-4d08-9a6c-1e7d0b8f42ac');
	});

	it('fr_mn_02_hides_the_signing_key_of_a_job_that_was_not_sent', async () => {
		mocks.getJob.mockResolvedValue(job({ signing_key_id: null }));
		render(JobDetailPage);

		await screen.findByText('About this job');
		expect(screen.queryByText('Signing key')).not.toBeInTheDocument();
	});

	it('shows one log inside the step, collapsed with its counts until it opens', async () => {
		const user = userEvent.setup();
		render(JobDetailPage);

		const toggle = await screen.findByRole('button', { name: 'Show the log of Backup' });
		await waitFor(() => expect(toggle).toHaveTextContent('2 lines · 1 WARN'));
		expect(screen.queryByRole('log')).not.toBeInTheDocument();
		expect(screen.queryByText('Live log')).not.toBeInTheDocument();

		await user.click(toggle);
		const log = await screen.findByRole('log', { name: 'Log of the step: Backup' });
		expect(within(log).getByText('snapshot a1b2c3d4 saved')).toBeInTheDocument();
		expect(within(log).getByText('Warning: a file changed')).toBeInTheDocument();
		expect(within(log).getByText('WARN')).toBeInTheDocument();
		expect(within(log).getByText('2')).toBeInTheDocument();
		expect(mocks.getJobLogs).toHaveBeenCalledWith('j_1', { after_seq: 0, limit: 1000 });
		expect(screen.queryByRole('button', { name: 'Pause' })).not.toBeInTheDocument();
	});

	it('filters the log of a step by level and by text', async () => {
		const user = userEvent.setup();
		render(JobDetailPage);

		await user.click(await screen.findByRole('button', { name: 'Show the log of Backup' }));
		const log = await screen.findByRole('log');
		await user.selectOptions(screen.getByRole('combobox', { name: 'Level' }), 'problems');
		expect(within(log).queryByText('snapshot a1b2c3d4 saved')).not.toBeInTheDocument();
		expect(within(log).getByText('Warning: a file changed')).toBeInTheDocument();
		expect(screen.getByText('1 of 2 lines')).toBeInTheDocument();

		await user.selectOptions(screen.getByRole('combobox', { name: 'Level' }), 'all');
		await user.type(screen.getByRole('searchbox', { name: 'Search the log' }), 'SNAPSHOT');
		expect(
			within(screen.getByRole('log')).getByText('snapshot a1b2c3d4 saved')
		).toBeInTheDocument();
		expect(screen.queryByText('Warning: a file changed')).not.toBeInTheDocument();
		await user.type(screen.getByRole('searchbox', { name: 'Search the log' }), 'zzz');
		expect(screen.getByText('No line matches the filter.')).toBeInTheDocument();
	});

	it('copies the lines that the filter shows', async () => {
		const user = userEvent.setup();
		const writeText = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue();
		render(JobDetailPage);

		await user.click(await screen.findByRole('button', { name: 'Show the log of Backup' }));
		await user.selectOptions(screen.getByRole('combobox', { name: 'Level' }), 'problems');
		await user.click(screen.getByRole('button', { name: 'Copy' }));
		expect(writeText).toHaveBeenCalledWith('2026-10-14T10:01:02Z WARN Warning: a file changed');
	});

	it('shows lines of no step in the general log and hides the block when it is empty', async () => {
		mocks.getJobLogs.mockResolvedValue(
			logPage([line(1, 'snapshot a1b2c3d4 saved'), { ...line(2, 'agent restarted'), step_id: '' }])
		);
		const { unmount } = render(JobDetailPage);

		const general = await screen.findByRole('log', { name: 'Log of the step: General log' });
		expect(within(general).getByText('agent restarted')).toBeInTheDocument();
		unmount();

		mocks.getJobLogs.mockResolvedValue(logPage([line(1, 'snapshot a1b2c3d4 saved')]));
		render(JobDetailPage);
		await screen.findByRole('button', { name: 'Show the log of Backup' });
		expect(screen.queryByText('General log')).not.toBeInTheDocument();
	});

	it('downloads the whole log as a text file through the logs API', async () => {
		const user = userEvent.setup();
		mocks.getJobLogs.mockImplementation(async (_id: string, query: { after_seq: number }) =>
			query.after_seq === 0
				? logPage([line(1, 'first')], { has_more: true })
				: logPage([line(2, 'second')])
		);
		const create = vi.fn(() => 'blob:log');
		const revoke = vi.fn();
		Object.assign(URL, { createObjectURL: create, revokeObjectURL: revoke });
		const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
		render(JobDetailPage);

		await user.click(await screen.findByRole('button', { name: 'Download log' }));
		await waitFor(() => expect(click).toHaveBeenCalled());
		const blob = (create.mock.calls[0] as unknown as [Blob])[0];
		expect(await blob.text()).toBe(
			'2026-10-14T10:01:02Z INFO first\n2026-10-14T10:01:02Z INFO second\n'
		);
		expect(mocks.getJobLogs).toHaveBeenLastCalledWith('j_1', { after_seq: 1, limit: 2000 });
		expect(revoke).toHaveBeenCalledWith('blob:log');
	});

	it('shows the progress of the running step and offers the cancel action', async () => {
		mocks.getJob.mockResolvedValue(
			job({
				status: 'running',
				result_code: null,
				finished_at: null,
				planned_steps: ['backup', 'forget', 'prune'],
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
		expect(screen.queryByText('Live log')).not.toBeInTheDocument();
		expect(await screen.findByText('The step wrote no log line.')).toBeInTheDocument();
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

	it('fr_ag_09_shows_why_restic_did_not_start_in_the_step_and_the_header', async () => {
		const reason =
			'cannot protect the directory C:\\ProgramData\\ResticHub; run `restichub-agent install-service` as an administrator';
		mocks.getJob.mockResolvedValue(
			job({
				status: 'failed',
				result_code: 'failed',
				result_message: 'step_not_started',
				summary: null,
				steps: [
					{
						step_id: 'backup',
						status: 'failed',
						exit_code: -1,
						started_at: '2026-10-14T10:01:00Z',
						finished_at: '2026-10-14T10:01:00Z',
						error_code: 'step_not_started',
						error_params: { reason },
						possible_stale_lock: false,
						output: { kind: 'none' }
					}
				]
			})
		);
		mocks.getJobLogs.mockResolvedValue(logPage([]));
		render(JobDetailPage);

		expect(
			await screen.findByText(`restic did not start. Agent message: ${reason}`)
		).toBeInTheDocument();
		expect(
			screen.getByText(`The step did not start. Agent message: ${reason}`)
		).toBeInTheDocument();
		expect(screen.queryByText(/did not start: restic did not start/)).not.toBeInTheDocument();
		expect(
			await screen.findByText('The step wrote no log line. The cause is above.')
		).toBeInTheDocument();
		expect(screen.queryByText('The step wrote no log line.')).not.toBeInTheDocument();
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

	it('draws one icon per step state with no frame around it', async () => {
		const step = (id: string, status: string) => ({
			...job().steps[0],
			step_id: id,
			status,
			output: null
		});
		mocks.getJob.mockResolvedValue(
			job({
				status: 'running',
				result_code: null,
				finished_at: null,
				planned_steps: ['a', 'b', 'c', 'd', 'e', 'f'],
				planned: ['a', 'b', 'c', 'd', 'e', 'f'].map((id) => ({
					step_id: id,
					kind: 'backup',
					name: id,
					phase: null
				})),
				steps: [
					step('a', 'success'),
					step('b', 'warning'),
					step('c', 'failed'),
					step('d', 'skipped')
				],
				summary: null,
				progress: null
			})
		);
		const { container } = render(JobDetailPage);
		await waitFor(() => expect(container.querySelectorAll('.timeline__marker')).toHaveLength(6));

		const icons = [...container.querySelectorAll('.timeline__marker svg')].map((svg) =>
			svg.getAttribute('data-icon')
		);
		expect(icons).toEqual([
			'check-circle',
			'alert',
			'x-circle',
			'circle-slash',
			'loader-circle',
			'clock'
		]);
	});

	it('fr_mn_02_shows_the_header_line_and_all_steps_of_a_running_job', async () => {
		mocks.getJob.mockResolvedValue(
			job({
				status: 'running',
				result_code: null,
				finished_at: null,
				planned_steps: ['backup', 'forget', 'prune'],
				steps: [],
				summary: null,
				progress: {
					step_id: 'backup',
					bytes_done: 1,
					total_bytes: 2,
					files_done: 1,
					total_files: 2,
					percent_done: 0.5,
					seconds_elapsed: 90,
					seconds_remaining: 90,
					current_files: [],
					updated_at: '2026-10-14T10:02:30Z'
				}
			})
		);
		render(JobDetailPage);

		const subtitle = (await screen.findByText(/triggered by the schedule/)) as HTMLElement;
		expect(subtitle.textContent?.replace(/\s+/g, ' ').trim()).toBe(
			'Plan nightly-web · host win-fs01 · triggered by the schedule · step 1 of 3'
		);
		expect(screen.getByRole('heading', { level: 1, name: 'j_1' })).toBeInTheDocument();
		// The status pill alone tells that the job runs (UI principles 3.3).
		expect(screen.queryByText('live')).not.toBeInTheDocument();
		expect(screen.getByText('0 of 3 complete')).toBeInTheDocument();
		// The steps that did not start show as queued.
		expect(screen.getAllByText('queued')).toHaveLength(2);
		expect(screen.getByText('Delete expired snapshots')).toBeInTheDocument();
		expect(screen.getByText('Prune')).toBeInTheDocument();
		expect(screen.getByRole('progressbar', { name: 'Steps' })).toHaveValue(0.5 / 3);
	});

	it('shows the own duration of each step and the trigger in the about panel', async () => {
		mocks.getJob.mockResolvedValue(
			job({
				planned_steps: ['backup', 'forget'],
				expires_at: '2026-10-14T12:00:00Z',
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
					},
					{
						step_id: 'forget',
						status: 'success',
						exit_code: 0,
						started_at: '2026-10-14T10:04:12Z',
						finished_at: '2026-10-14T10:04:16Z',
						error_code: null,
						error_params: {},
						possible_stale_lock: false,
						output: { kind: 'forget', groups: 1, kept: 14, removed: 2, truncated: false }
					}
				]
			})
		);
		render(JobDetailPage);

		expect(await screen.findByText('2 of 2 complete')).toBeInTheDocument();
		expect(screen.getByText(/Succeeded · 3m 12s · exit code 0/)).toBeInTheDocument();
		expect(screen.getByText(/Succeeded · 4s · exit code 0/)).toBeInTheDocument();
		const about = within(screen.getByText('About this job').closest('section') as HTMLElement);
		expect(about.getByText('Trigger')).toBeInTheDocument();
		expect(about.getByText('Schedule')).toBeInTheDocument();
		// A finished job shows neither an acceptance window nor a limit.
		expect(about.queryByText('Accept until')).not.toBeInTheDocument();
		expect(about.queryByText('Limit')).not.toBeInTheDocument();
	});

	it('ac_ag_12_shows_the_acceptance_window_of_a_waiting_job_and_the_limit_of_a_running_job', async () => {
		mocks.getJob.mockResolvedValue(
			job({ status: 'queued', expires_at: '2026-10-14T12:00:00Z', planned_steps: [], steps: [] })
		);
		const queued = render(JobDetailPage);
		const waiting = within(
			(await screen.findByText('About this job')).closest('section') as HTMLElement
		);
		expect(waiting.getByText('Accept until')).toBeInTheDocument();
		expect(waiting.queryByText('Expires')).not.toBeInTheDocument();
		expect(waiting.queryByText('Limit')).not.toBeInTheDocument();
		queued.unmount();

		mocks.getJob.mockResolvedValue(
			job({ status: 'running', expires_at: '2026-10-14T12:00:00Z', planned_steps: [], steps: [] })
		);
		render(JobDetailPage);
		const running = within(
			(await screen.findByText('About this job')).closest('section') as HTMLElement
		);
		expect(await running.findByText('Limit')).toBeInTheDocument();
		expect(running.getByText('No progress: 1h')).toBeInTheDocument();
		expect(running.queryByText('Accept until')).not.toBeInTheDocument();
	});

	it('shows the steps that did not run of a failed job as not run', async () => {
		mocks.getJob.mockResolvedValue(
			job({
				status: 'failed',
				result_code: 'config_error',
				summary: null,
				planned_steps: ['backup', 'forget'],
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

		expect(await screen.findByText('1 of 2 complete')).toBeInTheDocument();
		expect(screen.getByText('not run')).toBeInTheDocument();
	});

	it('offers Cancel job while the job is queued and not after it ends', async () => {
		mocks.getJob.mockResolvedValue(
			job({ status: 'queued', result_code: null, planned_steps: [], steps: [], summary: null })
		);
		const queued = render(JobDetailPage);
		expect(await screen.findByRole('button', { name: 'Cancel job' })).toBeInTheDocument();
		queued.unmount();

		mocks.getJob.mockResolvedValue(job());
		render(JobDetailPage);
		await screen.findByText('Summary');
		expect(screen.queryByRole('button', { name: 'Cancel job' })).not.toBeInTheDocument();
	});

	it('runs the plan again and opens the new job', async () => {
		const user = userEvent.setup();
		mocks.runPlan.mockResolvedValue(job({ id: 'j_2', status: 'queued' }));
		render(JobDetailPage);

		await user.click(await screen.findByRole('button', { name: 'Run again' }));

		await waitFor(() => expect(mocks.runPlan).toHaveBeenCalledWith('pl_1'));
		await waitFor(() => expect(mocks.goto).toHaveBeenCalledWith('/(app)/jobs/j_2'));
	});

	it('disables Run again while the job runs and hides it without a plan or the right', async () => {
		mocks.getJob.mockResolvedValue(job({ status: 'running', result_code: null }));
		const running = render(JobDetailPage);
		expect(await screen.findByRole('button', { name: 'Run again' })).toBeDisabled();
		running.unmount();

		mocks.getJob.mockResolvedValue(job({ plan_id: null, plan_name: null }));
		const manual = render(JobDetailPage);
		await screen.findByText('Summary');
		expect(screen.queryByRole('button', { name: 'Run again' })).not.toBeInTheDocument();
		manual.unmount();

		session.role = 'viewer';
		mocks.getJob.mockResolvedValue(job());
		render(JobDetailPage);
		await screen.findByText('Summary');
		expect(screen.queryByRole('button', { name: 'Run again' })).not.toBeInTheDocument();
	});

	it('fr_lg_01_asks_for_no_line_once_the_viewer_is_full', async () => {
		mocks.getJobLogs.mockImplementation(
			async (_id: string, query: { after_seq: number; limit: number }) => ({
				items: Array.from({ length: query.limit }, (_, index) =>
					line(query.after_seq + index + 1, 'x')
				),
				has_more: true,
				truncated: false,
				purged: false
			})
		);
		render(JobDetailPage);

		await screen.findByText(/The viewer shows the first 10,000 lines/);
		expect(mocks.getJobLogs).toHaveBeenCalledTimes(10);
		for (const [, query] of mocks.getJobLogs.mock.calls) expect(query.limit).toBe(1000);
	}, 60_000);

	function scriptStep(
		id: string,
		name: string,
		phase: string,
		overrides: Record<string, unknown> = {}
	) {
		return {
			step_id: id,
			kind: 'script',
			name,
			phase,
			status: 'success',
			exit_code: 0,
			started_at: '2026-10-14T10:01:00Z',
			finished_at: '2026-10-14T10:01:42Z',
			error_code: null,
			error_params: {},
			possible_stale_lock: false,
			output: { kind: 'none' },
			...overrides
		};
	}

	function stepLine(seq: number, stepId: string, text: string) {
		return { ...line(seq, text), step_id: stepId };
	}

	function scriptedJob(overrides: Record<string, unknown> = {}) {
		const first = scriptStep('script-0', 'Dump the database', 'before');
		const backup = { ...job().steps[0] };
		const last = scriptStep('script-1', 'Remove old dumps', 'finally');
		return job({
			planned_steps: ['script-0', 'backup', 'script-1'],
			planned: [
				{ step_id: 'script-0', kind: 'script', name: 'Dump the database', phase: 'before' },
				{ step_id: 'backup', kind: 'backup', name: '', phase: null },
				{ step_id: 'script-1', kind: 'script', name: 'Remove old dumps', phase: 'finally' }
			],
			steps: [first, backup, last],
			...overrides
		});
	}

	// FR-HK-11: every script is a step with its own name, phase and log.
	it('fr_hk_11_lists_each_script_as_a_step_with_its_name_and_phase', async () => {
		mocks.getJob.mockResolvedValue(scriptedJob());
		render(JobDetailPage);

		const dump = await screen.findByRole('button', { name: 'Show the log of Dump the database' });
		expect(dump).toHaveTextContent('Script');
		expect(dump).toHaveTextContent('Before');
		expect(dump).toHaveTextContent('Succeeded');
		const cleanup = screen.getByRole('button', { name: 'Show the log of Remove old dumps' });
		expect(cleanup).toHaveTextContent('Finally');
	});

	it('fr_hk_11_shows_the_lines_of_each_step_in_its_own_log', async () => {
		const user = userEvent.setup();
		mocks.getJob.mockResolvedValue(scriptedJob());
		mocks.getJobLogs.mockResolvedValue(
			logPage([
				stepLine(1, 'script-0', 'dump written: /var/backups/app.dump'),
				stepLine(2, 'backup', 'snapshot a1b2c3d4 saved')
			])
		);
		render(JobDetailPage);

		await user.click(
			await screen.findByRole('button', { name: 'Show the log of Dump the database' })
		);
		const log = await screen.findByTestId('step-log-script-0');
		expect(within(log).getByText('dump written: /var/backups/app.dump')).toBeInTheDocument();
		expect(within(log).queryByText('snapshot a1b2c3d4 saved')).not.toBeInTheDocument();
		// One read serves every step: no request per step.
		expect(mocks.getJobLogs).toHaveBeenCalledTimes(1);
		// The step closes again.
		await user.click(screen.getByRole('button', { name: 'Hide the log of Dump the database' }));
		expect(screen.queryByTestId('step-log-script-0')).not.toBeInTheDocument();
	});

	it('fr_hk_11_opens_a_failed_script_by_itself_and_shows_its_exit_code', async () => {
		mocks.getJob.mockResolvedValue(
			scriptedJob({
				status: 'failed',
				result_code: 'failed',
				steps: [
					scriptStep('script-0', 'Dump the database', 'before', {
						status: 'failed',
						exit_code: 7
					})
				]
			})
		);
		mocks.getJobLogs.mockResolvedValue(
			logPage([stepLine(1, 'script-0', 'pg_dump: connection refused')])
		);
		render(JobDetailPage);

		const log = await screen.findByTestId('step-log-script-0');
		expect(await within(log).findByText('pg_dump: connection refused')).toBeInTheDocument();
		const toggle = screen.getByRole('button', { name: 'Hide the log of Dump the database' });
		expect(toggle).toHaveTextContent('Failed');
		expect(toggle).toHaveTextContent('exit code 7');
	});

	it('fr_hk_11_says_that_a_step_wrote_no_log', async () => {
		const user = userEvent.setup();
		mocks.getJob.mockResolvedValue(scriptedJob());
		mocks.getJobLogs.mockResolvedValue(logPage([line(1, 'x')]));
		render(JobDetailPage);

		await user.click(
			await screen.findByRole('button', { name: 'Show the log of Remove old dumps' })
		);
		expect(await screen.findByText('The step wrote no log line.')).toBeInTheDocument();
	});

	it('does not offer a log for a script that did not start', async () => {
		mocks.getJob.mockResolvedValue(
			scriptedJob({ status: 'failed', result_code: 'failed', steps: [] })
		);
		render(JobDetailPage);

		expect(await screen.findByText('Dump the database')).toBeInTheDocument();
		expect(
			screen.queryByRole('button', { name: 'Show the log of Dump the database' })
		).not.toBeInTheDocument();
	});
});
