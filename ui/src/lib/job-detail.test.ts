/*
 * The statistics of the job detail come from the restic summary and are never recomputed
 * (FR-MN-02, AC-MN-04).
 */
import { describe, expect, it } from 'vitest';
import type { StepOutput } from '#lib/api/index.ts';
import { isLive, markerOf, statsOf } from './job-detail';

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
		expect(processed?.params).toEqual({ files: '115' });
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
