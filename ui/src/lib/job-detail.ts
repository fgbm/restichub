/*
 * Pure helpers of the job detail screen: which jobs poll, how a step looks, and which statistics a
 * summary shows (FR-MN-02). The values come from the restic summary and are never recomputed.
 */
import { formatBytes, formatCount, formatSeconds } from '#lib/format.ts';
import type { JobStatus, JobStep, StepOutput } from '#lib/api/index.ts';
import type { MessageKey } from '#lib/i18n/index.ts';
import type { StatusTone } from '#lib/components/ui/index.ts';

/** A job in one of these states can still change, so the screen polls. */
export function isLive(status: JobStatus): boolean {
	return status === 'queued' || status === 'sent' || status === 'running';
}

export type StepMarker = 'success' | 'warning' | 'running' | 'failed' | 'idle';

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
			return 'idle';
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
					params: { files: formatCount(output.total_files_processed) }
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
		default:
			return [];
	}
}
