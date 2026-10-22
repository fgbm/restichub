/*
 * Pure helpers of the job detail screen: which jobs poll, how a step looks, and which statistics a
 * summary shows (FR-MN-02). The values come from the restic summary and are never recomputed.
 */
import { formatBytes, formatCount, formatSeconds } from '#lib/format.ts';
import type { JobDetail, JobStatus, JobStep, StepOutput } from '#lib/api/index.ts';
import { t } from '#lib/i18n/index.ts';
import { pluralRules } from '#lib/intl.ts';
import type { MessageKey } from '#lib/i18n/index.ts';
import type { StatusTone } from '#lib/components/ui/index.ts';

/** A job in one of these states can still change, so the screen polls. */
export function isLive(status: JobStatus): boolean {
	return status === 'queued' || status === 'sent' || status === 'running';
}

/**
 * The stall timeout of a restore that the control plane sends without one. The agent stops a step
 * after this time without progress, unless its own `RESTICHUB_JOB_STALL_TIMEOUT_SECS` changes it
 * (ADR-0039).
 */
const RESTORE_DEFAULT_STALL_SECONDS = 3600;

/**
 * The limits of a running job as lines for the about panel (ADR-0039). A run has no fixed time
 * limit by default: the host stops a step that makes no progress, and a profile can set a maximum
 * duration. Null for a job that does not run.
 */
export function limitLines(job: JobDetail): string[] | null {
	if (job.status !== 'running') return null;
	const { max_duration_seconds: max, stall_timeout_seconds: stall } = job.limits;
	const lines = [
		max ? t('jobDetail.limit.max', { max: formatSeconds(max) }) : t('jobDetail.limit.noMax')
	];
	if (stall) lines.push(t('jobDetail.limit.stall', { stall: formatSeconds(stall) }));
	else if (job.kind === 'restore')
		lines.push(
			t('jobDetail.limit.stallDefault', {
				stall: formatSeconds(RESTORE_DEFAULT_STALL_SECONDS)
			})
		);
	else if (!max) lines.push(t('jobDetail.limit.noProgress'));
	return lines;
}

/** The count of files with the plural form of the current language, such as "128 402 файла". */
export function formatFiles(count: number): string {
	const rule = pluralRules().select(count);
	return t(`jobDetail.files.${rule}` as MessageKey, { count: formatCount(count) });
}

/** The first part of a signing key identifier; the full value stays in the title and the copy. */
export function shortKeyId(keyId: string): string {
	return keyId.slice(0, 8);
}

/** One step of the list: the result of a step, the running step or a step that did not start. */
export interface StepRow {
	id: string;
	state: 'done' | 'running' | 'pending';
	step: JobStep | null;
	/** The kind of the step, such as `script`, `hook` or `backup`. Empty for an older job. */
	kind: string;
	/** The name of a script or the identifier of a hook, or empty. */
	name: string;
	/** The phase of a hook or a script, or empty. */
	phase: string;
}

/**
 * All steps of a job in run order. The control plane knows the list when it signs the job, so the
 * steps that did not start show as well. A result for a step that the list lacks follows the list.
 */
export function stepRows(job: JobDetail): StepRow[] {
	const results = new Map(job.steps.map((step) => [step.step_id, step]));
	const ids = [...job.planned_steps];
	for (const step of job.steps) if (!ids.includes(step.step_id)) ids.push(step.step_id);
	const pending = ids.filter((id) => !results.has(id));
	const progressId = job.progress?.step_id;
	// The agent runs the steps in order, so without progress the next one in the list runs.
	const runningId =
		job.status === 'running'
			? progressId && pending.includes(progressId)
				? progressId
				: (pending[0] ?? null)
			: null;
	const described = new Map((job.planned ?? []).map((entry) => [entry.step_id, entry]));
	return ids.map((id) => {
		const step = results.get(id) ?? null;
		const entry = described.get(id);
		const kind = step?.kind || entry?.kind || '';
		const name = step?.name || entry?.name || '';
		const phase = step?.phase || entry?.phase || '';
		if (step) return { id, state: 'done', step, kind, name, phase };
		return { id, state: id === runningId ? 'running' : 'pending', step: null, kind, name, phase };
	});
}

export interface StepProgressSummary {
	complete: number;
	total: number;
	/** The step that runs or runs next, counted from 1. */
	position: number;
	/** From 0 to 1: the finished steps and the progress of the running step. */
	fraction: number;
}

export function stepProgress(job: JobDetail, rows: StepRow[] = stepRows(job)): StepProgressSummary {
	const total = rows.length;
	const complete = rows.filter((row) => row.state === 'done').length;
	const running = rows.find((row) => row.state === 'running');
	const partial = running && job.progress?.step_id === running.id ? job.progress.percent_done : 0;
	return {
		complete,
		total,
		position: Math.min(total, complete + 1),
		fraction: total === 0 ? 0 : Math.min(1, (complete + partial) / total)
	};
}

export type StepMarker = 'success' | 'warning' | 'running' | 'failed' | 'skipped' | 'idle' | 'info';

/** restic exit code 10: no repository at the location. */
const NO_REPOSITORY = 10;

/**
 * The access test of an init job that found no repository. It is the expected answer before the
 * init step, not a failure (FR-RP-02).
 */
export function isInitProbe(job: Pick<JobDetail, 'kind'>, step: JobStep): boolean {
	return job.kind === 'init' && step.step_id === 'test-access' && step.exit_code === NO_REPOSITORY;
}

/**
 * The error code of a step. An older agent sends `step_config_error` for an access test that
 * found no repository, so the exit code decides.
 */
export function stepErrorCode(step: JobStep): string | null | undefined {
	if (step.step_id === 'test-access' && step.exit_code === NO_REPOSITORY) {
		return 'repository_missing';
	}
	return step.error_code;
}

export function markerOf(status: JobStep['status']): StepMarker {
	switch (status) {
		case 'success':
			return 'success';
		case 'warning':
		case 'retry':
			return 'warning';
		case 'failed':
		case 'config_error':
			return 'failed';
		default:
			return 'skipped';
	}
}

export function toneOfStep(status: JobStep['status']): StatusTone {
	switch (markerOf(status)) {
		case 'success':
			return 'success';
		case 'warning':
			return 'warning';
		case 'failed':
			return 'failed';
		default:
			return 'cancelled';
	}
}

export interface Stat {
	label: MessageKey;
	value: string;
	mono?: boolean;
	/** Values with a parameter, such as the processed files, give the parameters. */
	params?: Record<string, string>;
}

/** The statistics that a summary shows, in display order. */
export function statsOf(output: StepOutput | null | undefined): Stat[] {
	if (!output) return [];
	switch (output.kind) {
		case 'backup':
			return [
				{ label: 'jobDetail.stat.dataAdded', value: formatBytes(output.data_added) },
				{ label: 'jobDetail.stat.filesNew', value: formatCount(output.files_new) },
				{ label: 'jobDetail.stat.duration', value: formatSeconds(output.total_duration_seconds) },
				{
					label: 'jobDetail.stat.snapshot',
					value: output.snapshot_id || '—',
					mono: true
				},
				{ label: 'jobDetail.stat.filesChanged', value: formatCount(output.files_changed) },
				{ label: 'jobDetail.stat.filesUnchanged', value: formatCount(output.files_unmodified) },
				{
					label: 'jobDetail.stat.processed',
					value: formatBytes(output.total_bytes_processed),
					params: { files: formatFiles(output.total_files_processed) }
				},
				{ label: 'jobDetail.stat.dataPacked', value: formatBytes(output.data_added_packed) }
			];
		case 'check':
			return [
				{ label: 'jobDetail.stat.errors', value: formatCount(output.num_errors) },
				{ label: 'jobDetail.stat.blobsChecked', value: formatCount(output.blobs_checked) },
				{ label: 'jobDetail.stat.packsChecked', value: formatCount(output.packs_checked) }
			];
		case 'forget':
			return [
				{ label: 'jobDetail.stat.groups', value: formatCount(output.groups) },
				{ label: 'jobDetail.stat.kept', value: formatCount(output.kept) },
				{ label: 'jobDetail.stat.removed', value: formatCount(output.removed) }
			];
		case 'snapshots':
			return [{ label: 'jobDetail.stat.snapshots', value: formatCount(output.count) }];
		case 'stats':
			return [
				{ label: 'jobDetail.stat.totalSize', value: formatBytes(output.total_size) },
				{
					label: 'jobDetail.stat.uncompressed',
					value: formatBytes(output.total_uncompressed_size)
				},
				{ label: 'jobDetail.stat.fileCount', value: formatCount(output.total_file_count) },
				{ label: 'jobDetail.stat.snapshots', value: formatCount(output.snapshots_count) },
				{ label: 'jobDetail.stat.ratio', value: output.compression_ratio.toFixed(2) }
			];
		case 'restore':
			return [
				{ label: 'jobDetail.stat.filesRestored', value: formatCount(output.files_restored) },
				{ label: 'jobDetail.stat.bytesRestored', value: formatBytes(output.bytes_restored) },
				{ label: 'jobDetail.stat.duration', value: formatSeconds(output.seconds_elapsed) },
				{
					label: 'jobDetail.stat.dryRun',
					value: t(output.dry_run ? 'common.yes' : 'common.no')
				},
				{ label: 'jobDetail.stat.filesSkipped', value: formatCount(output.files_skipped) },
				{ label: 'jobDetail.stat.bytesSkipped', value: formatBytes(output.bytes_skipped) }
			];
		default:
			return [];
	}
}
