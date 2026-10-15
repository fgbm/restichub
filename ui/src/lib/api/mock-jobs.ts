/*
 * Sample step results, progress and log lines of the mock jobs (FR-MN-02, FR-LG-01).
 *
 * A function derives all values from the job and the clock, so a sample job needs no stored state.
 * A running job grows by one log line every few seconds and moves its progress, so the screen
 * shows live following without a server.
 */
import type {
	Job,
	JobDetail,
	JobLogLine,
	JobLogPage,
	JobLogQuery,
	JobProgress,
	JobStep
} from './types';

const LINE_INTERVAL_MS = 4000;
const RUNNING_FILES = 128_402;
const RUNNING_BYTES = 1_400_000_000;
const RUNNING_SECONDS = 180;
const FORGET_SECONDS = 4;

type Level = JobLogLine['level'];
type Stream = JobLogLine['stream'];
type Draft = [Level, string, Stream];

const BACKUP_SUMMARY = {
	kind: 'backup' as const,
	files_new: 128,
	files_changed: 14,
	files_unmodified: 128_260,
	dirs_new: 6,
	dirs_changed: 3,
	dirs_unmodified: 9_811,
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

const RESTORE_SUMMARY = {
	kind: 'restore' as const,
	total_files: 4_210,
	files_restored: 4_187,
	files_skipped: 23,
	files_deleted: 0,
	total_bytes: 3_914_880_000,
	bytes_restored: 3_902_113_792,
	bytes_skipped: 12_766_208,
	seconds_elapsed: 41.6,
	dry_run: false
};

function stepIdOf(job: Job): string {
	return job.kind === 'restore' ? 'restore' : 'backup';
}

function startOf(job: Job): number {
	return Date.parse(job.started_at ?? job.created_at);
}

function isWarning(job: Job): boolean {
	return job.result_code === 'succeeded_with_warnings';
}

function openingLines(job: Job): Draft[] {
	if (job.kind === 'restore') {
		return [
			['info', `job ${job.id} accepted on ${job.agent_hostname ?? 'the host'}`, 'agent'],
			['info', `repository ${job.repository_name ?? ''} is open`, 'agent'],
			['info', 'restoring snapshot a1b2c3d4 to /srv/restore', 'stdout']
		];
	}
	return [
		['info', `job ${job.id} accepted on ${job.agent_hostname ?? 'the host'}`, 'agent'],
		['info', `repository ${job.repository_name ?? ''} is open`, 'agent'],
		['info', 'using parent snapshot 7f3c9a10', 'stdout'],
		['info', 'start scan on [C:\\Data]', 'stdout'],
		['info', 'start backup on [C:\\Data]', 'stdout'],
		['info', 'scan finished in 38.2s: 128402 files, 49.28 GiB', 'stdout']
	];
}

function closingLines(job: Job): Draft[] {
	const lines: Draft[] = [];
	if (isWarning(job)) {
		lines.push(['warn', 'Warning: at least one source file could not be read', 'stderr']);
		lines.push(['warn', 'file C:\\Data\\locks\\.db-shm changed during backup', 'stderr']);
	}
	if (job.result_code === 'config_error') {
		lines.push(['error', 'Fatal: wrong password or no key found', 'stderr']);
		return lines;
	}
	if (job.kind === 'restore' && job.status === 'succeeded') {
		lines.push(['info', 'Summary: Restored 4187 files/dirs (3.634 GiB) in 0:41', 'stdout']);
		lines.push(['info', `job ${job.id} finished`, 'agent']);
		return lines;
	}
	if (job.status === 'failed' || job.status === 'dead_letter') {
		lines.push(['error', 'Fatal: unable to open the repository lock', 'stderr']);
		return lines;
	}
	lines.push(['info', 'snapshot a1b2c3d4 saved', 'stdout']);
	lines.push(['info', `job ${job.id} finished`, 'agent']);
	return lines;
}

/** All lines a job holds at the time `now`. */
export function logLinesOf(job: Job, now: number): JobLogLine[] {
	if (job.status === 'queued' || job.status === 'sent') return [];
	const start = startOf(job);
	const lines: JobLogLine[] = [];
	const push = (draft: Draft, at: number) =>
		lines.push({
			seq: lines.length + 1,
			step_id: stepIdOf(job),
			stream: draft[2],
			level: draft[0],
			ts: new Date(at).toISOString(),
			text: draft[1]
		});
	openingLines(job).forEach((draft, index) => push(draft, start + index * 2000));
	const middle =
		job.status === 'running' ? Math.max(0, Math.floor((now - start) / LINE_INTERVAL_MS) - 6) : 24;
	for (let index = 0; index < Math.min(middle, 400); index += 1) {
		const files = 1200 * (index + 1);
		push(
			['info', `${files} files processed, ${(files * 0.012).toFixed(1)} GiB added`, 'stdout'],
			start + (index + 6) * LINE_INTERVAL_MS
		);
	}
	if (job.status !== 'running') {
		const end = Date.parse(job.finished_at ?? job.updated_at);
		closingLines(job).forEach((draft, index) => push(draft, end - 2000 + index * 500));
	}
	return lines;
}

/** One page of the lines after a line number. */
export function logPageOf(job: Job, query: JobLogQuery | undefined, now: number): JobLogPage {
	const after = query?.after_seq ?? 0;
	const limit = query?.limit ?? 500;
	const rest = logLinesOf(job, now).filter((item) => item.seq > after);
	return {
		items: rest.slice(0, limit),
		has_more: rest.length > limit,
		truncated: false,
		purged: false
	};
}

function progressOf(job: Job, now: number): JobProgress | null {
	if (job.status !== 'running') return null;
	const elapsed = Math.max(1, (now - startOf(job)) / 1000);
	const percent = Math.min(0.96, elapsed / RUNNING_SECONDS);
	return {
		step_id: stepIdOf(job),
		bytes_done: Math.round(RUNNING_BYTES * percent),
		total_bytes: RUNNING_BYTES,
		files_done: Math.round(RUNNING_FILES * percent),
		total_files: RUNNING_FILES,
		percent_done: percent,
		seconds_elapsed: elapsed,
		seconds_remaining: Math.max(1, Math.round(RUNNING_SECONDS - elapsed)),
		current_files: ['C:\\Data\\projects\\site\\assets\\hero-large.psd'],
		updated_at: new Date(now).toISOString()
	};
}

function failedStep(job: Job, status: JobStep['status'], exit: number, code: string): JobStep {
	return {
		step_id: stepIdOf(job),
		status,
		exit_code: exit,
		started_at: job.started_at ?? job.created_at,
		finished_at: job.finished_at ?? job.updated_at,
		error_code: code,
		error_params: {},
		possible_stale_lock: false,
		output: { kind: 'none' }
	};
}

function stepsOf(job: Job): JobStep[] {
	if (job.status === 'queued' || job.status === 'sent' || job.status === 'running') return [];
	if (job.result_code === 'config_error') {
		return [failedStep(job, 'config_error', 12, 'step_config_error')];
	}
	if (job.status === 'failed' || job.status === 'dead_letter') {
		return [failedStep(job, 'retry', 11, 'repository_locked')];
	}
	const started = job.started_at ?? job.created_at;
	if (job.kind === 'restore') {
		return [
			{
				step_id: 'restore',
				status: 'success',
				exit_code: 0,
				started_at: started,
				finished_at: new Date(
					Date.parse(started) + RESTORE_SUMMARY.seconds_elapsed * 1000
				).toISOString(),
				error_code: null,
				error_params: {},
				possible_stale_lock: false,
				output: RESTORE_SUMMARY
			}
		];
	}
	// Each step has its own time span: the backup takes the time of its summary, the forget a few
	// seconds after it.
	const backupEnd = Date.parse(started) + BACKUP_SUMMARY.total_duration_seconds * 1000;
	const middle = new Date(backupEnd).toISOString();
	const finished = new Date(backupEnd + FORGET_SECONDS * 1000).toISOString();
	return [
		{
			step_id: 'backup',
			status: isWarning(job) ? 'warning' : 'success',
			exit_code: isWarning(job) ? 3 : 0,
			started_at: started,
			finished_at: middle,
			error_code: isWarning(job) ? 'step_warning' : null,
			error_params: {},
			possible_stale_lock: false,
			output: BACKUP_SUMMARY
		},
		{
			step_id: 'forget',
			status: 'success',
			exit_code: 0,
			started_at: middle,
			finished_at: finished,
			error_code: null,
			error_params: {},
			possible_stale_lock: false,
			output: { kind: 'forget', groups: 1, kept: 14, removed: 2, truncated: false }
		}
	];
}

/** The steps that the control plane signed into the job. A queued job has no list yet. */
function plannedStepsOf(job: Job): string[] {
	if (job.status === 'queued') return [];
	if (job.kind === 'restore') return ['restore'];
	if (job.status === 'running') return ['backup', 'forget', 'prune'];
	return ['backup', 'forget'];
}

/** The detail of a job at the time `now`: steps, summary, progress and the state of the log. */
export function detailOf(job: Job, now: number): JobDetail {
	const steps = stepsOf(job);
	const summary = steps.map((step) => step.output).find((output) => output.kind !== 'none') ?? null;
	return {
		...job,
		trigger: job.kind === 'restore' ? 'manual' : 'schedule',
		planned_steps: plannedStepsOf(job),
		steps,
		summary,
		progress: progressOf(job, now),
		log: { lines: logLinesOf(job, now).length, truncated: false, purged: false }
	};
}
