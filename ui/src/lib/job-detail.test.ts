/*
 * The statistics of the job detail come from the restic summary and are never recomputed
 * (FR-MN-02, AC-MN-04).
 */
import { afterEach, describe, expect, it } from 'vitest';
import type { JobDetail, StepOutput } from '#lib/api/index.ts';
import { setLocale, t } from '#lib/i18n/index.ts';
import {
	formatFiles,
	isLive,
	limitText,
	markerOf,
	statsOf,
	stepProgress,
	stepRows
} from './job-detail';

afterEach(() => setLocale('en'));

const summary: StepOutput = {
	kind: 'backup',
	files_new: 12,
	files_changed: 3,
	files_unmodified: 100,
	dirs_new: 2,
	dirs_changed: 1,
	dirs_unmodified: 9,
	data_blobs: 14,
	tree_blobs: 3,
	data_added: 1_503_238_553,
	data_added_packed: 1_000,
	total_files_processed: 115,
	total_bytes_processed: 4096,
	total_duration_seconds: 192.4,
	snapshot_id: 'a1b2c3d4',
	dry_run: false
};

function byLabel(output: StepOutput) {
	return Object.fromEntries(statsOf(output).map((stat) => [stat.label, stat.value]));
}

describe('statsOf', () => {
	it('fr_rs_04_shows_the_summary_of_a_restore', () => {
		const stats = Object.fromEntries(
			statsOf({
				kind: 'restore',
				total_files: 10,
				files_restored: 8,
				files_skipped: 2,
				files_deleted: 0,
				total_bytes: 3_000,
				bytes_restored: 2_048,
				bytes_skipped: 952,
				seconds_elapsed: 12,
				dry_run: true
			}).map((stat) => [stat.label, stat.value])
		);
		expect(stats['jobDetail.stat.filesRestored']).toBe('8');
		expect(stats['jobDetail.stat.bytesRestored']).toBe('2 KiB');
		expect(stats['jobDetail.stat.filesSkipped']).toBe('2');
		expect(stats['jobDetail.stat.dryRun']).toBe('Yes');
	});

	it('ac_mn_04_shows_the_values_of_the_restic_backup_summary', () => {
		const stats = byLabel(summary);
		expect(stats['jobDetail.stat.dataAdded']).toBe('1.4 GiB');
		expect(stats['jobDetail.stat.filesNew']).toBe('12');
		expect(stats['jobDetail.stat.filesChanged']).toBe('3');
		expect(stats['jobDetail.stat.filesUnchanged']).toBe('100');
		expect(stats['jobDetail.stat.duration']).toBe('3m 12s');
		expect(stats['jobDetail.stat.snapshot']).toBe('a1b2c3d4');
		expect(stats['jobDetail.stat.dataPacked']).toBe('1,000 B');
		const processed = statsOf(summary).find((stat) => stat.label === 'jobDetail.stat.processed');
		expect(processed?.value).toBe('4 KiB');
		expect(processed?.params).toEqual({ files: '115 files' });
	});

	it('shows a dash for the snapshot of a dry run', () => {
		const stats = byLabel({ ...(summary as object), snapshot_id: '', dry_run: true } as StepOutput);
		expect(stats['jobDetail.stat.snapshot']).toBe('—');
	});

	it('shows the counts of a check and a forget step', () => {
		expect(
			byLabel({
				kind: 'check',
				num_errors: 2,
				repair_suggested: true,
				blobs_checked: 9000,
				packs_checked: 40
			})
		).toEqual({
			'jobDetail.stat.errors': '2',
			'jobDetail.stat.blobsChecked': '9,000',
			'jobDetail.stat.packsChecked': '40'
		});
		expect(byLabel({ kind: 'forget', groups: 1, kept: 14, removed: 2, truncated: false })).toEqual({
			'jobDetail.stat.groups': '1',
			'jobDetail.stat.kept': '14',
			'jobDetail.stat.removed': '2'
		});
	});

	it('shows nothing for a step without a summary', () => {
		expect(statsOf({ kind: 'none' })).toEqual([]);
		expect(statsOf(null)).toEqual([]);
	});
});

describe('the job state', () => {
	it('polls the jobs that can still change', () => {
		expect(isLive('queued')).toBe(true);
		expect(isLive('sent')).toBe(true);
		expect(isLive('running')).toBe(true);
		expect(isLive('succeeded')).toBe(false);
		expect(isLive('dead_letter')).toBe(false);
	});

	it('marks a locked repository as a warning and a bad configuration as a failure', () => {
		expect(markerOf('retry')).toBe('warning');
		expect(markerOf('config_error')).toBe('failed');
		expect(markerOf('success')).toBe('success');
		expect(markerOf('skipped')).toBe('idle');
	});
});

describe('the Russian wording', () => {
	it('shows the processed files with the plural form and no prefix', () => {
		setLocale('ru');
		const big = { ...(summary as object), total_files_processed: 128_402 } as StepOutput;
		const processed = statsOf(big).find((stat) => stat.label === 'jobDetail.stat.processed');
		const text = t('jobDetail.stat.processedValue', { ...processed?.params, size: '49,3 GiB' });
		expect(text.replace(/\u00a0/g, ' ')).toBe('128 402 файла · 49,3 GiB');
		expect(formatFiles(1)).toBe('1 файл');
		expect(formatFiles(2)).toBe('2 файла');
		expect(formatFiles(5)).toBe('5 файлов');
		expect(formatFiles(11)).toBe('11 файлов');
		expect(formatFiles(21)).toBe('21 файл');
	});

	it('names the forget step as removing snapshots', () => {
		setLocale('ru');
		expect(t('jobs.kind.forget')).toBe('Удаление снимков (forget)');
	});
});

function detail(overrides: Partial<JobDetail>): JobDetail {
	return {
		status: 'running',
		planned_steps: ['backup', 'forget', 'prune'],
		steps: [],
		progress: null,
		...overrides
	} as unknown as JobDetail;
}

const done = (step_id: string) =>
	({
		step_id,
		status: 'success',
		output: { kind: 'none' }
	}) as unknown as JobDetail['steps'][number];

describe('the step list', () => {
	it('fr_mn_02_lists_the_planned_steps_that_did_not_start', () => {
		const job = detail({
			steps: [done('backup')],
			progress: { step_id: 'forget', percent_done: 0.5 } as JobDetail['progress']
		});
		expect(stepRows(job).map((row) => [row.id, row.state])).toEqual([
			['backup', 'done'],
			['forget', 'running'],
			['prune', 'pending']
		]);
		expect(stepProgress(job)).toEqual({
			complete: 1,
			total: 3,
			position: 2,
			fraction: 1.5 / 3
		});
	});

	it('takes the next step as running without progress and keeps a result outside the list', () => {
		const job = detail({ steps: [done('backup'), done('extra')] });
		expect(stepRows(job).map((row) => [row.id, row.state])).toEqual([
			['backup', 'done'],
			['forget', 'running'],
			['prune', 'pending'],
			['extra', 'done']
		]);
	});

	it('shows no running step in a job that ended', () => {
		const job = detail({ status: 'failed', steps: [done('backup')] });
		expect(stepRows(job).map((row) => row.state)).toEqual(['done', 'pending', 'pending']);
		expect(stepProgress(job).complete).toBe(1);
	});
});

describe('limitText', () => {
	const limits = (max: number | null, stall: number | null) =>
		({ max_duration_seconds: max, stall_timeout_seconds: stall }) as JobDetail['limits'];

	it('ac_ag_12_says_that_a_running_job_has_no_time_limit_and_stops_without_progress', () => {
		expect(limitText(detail({ limits: limits(null, 3600) }))).toBe(
			'none · stops after 1h without progress'
		);
		expect(limitText(detail({ limits: limits(null, null) }))).toBe(
			'none · stops when restic makes no progress'
		);
	});

	it('ac_ag_13_shows_the_maximum_duration_when_the_profile_sets_one', () => {
		expect(limitText(detail({ limits: limits(7200, null) }))).toBe('stops after 2h');
		expect(limitText(detail({ limits: limits(7200, 3600) }))).toBe(
			'stops after 2h · or after 1h without progress'
		);
	});

	it('ac_ag_12_shows_no_limit_for_a_job_that_does_not_run', () => {
		expect(limitText(detail({ status: 'queued', limits: limits(null, 3600) }))).toBeNull();
		expect(limitText(detail({ status: 'succeeded', limits: limits(null, 3600) }))).toBeNull();
	});

	it('ac_ag_12_speaks_russian', () => {
		setLocale('ru');
		expect(limitText(detail({ limits: limits(null, 3600) }))).toBe(
			'нет · остановка после 1 ч без прогресса'
		);
	});
});
