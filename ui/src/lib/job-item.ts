/*
 * The list item of a job: the jobs list, the recent jobs of a host and the overview use one shape
 * (design/style/ui-principles.md, 2).
 */
import type { Job } from '#lib/api/index.ts';
import type { ListMeta } from '#lib/components/ui/index.ts';
import { t } from '#lib/i18n/index.ts';
import type { MessageKey } from '#lib/i18n/index.ts';
import { dayTime, elapsed, secondsSpan } from '#lib/list-format.ts';

/** The plan name; a job without a plan shows its kind. */
export function jobTitle(job: Pick<Job, 'plan_name' | 'kind'>): string {
	return job.plan_name ?? t(`jobs.kind.${job.kind}` as MessageKey);
}

/** Line 2: the host, the start, the kind when the title is the plan, the repository, the attempt. */
export function jobMeta(job: Job, { host = true }: { host?: boolean } = {}): ListMeta[] {
	const at = job.started_at ?? job.run_at;
	return [
		host ? (job.agent_hostname ?? '') : null,
		at ? { text: dayTime(at), time: at } : null,
		job.plan_name ? t(`jobs.kind.${job.kind}` as MessageKey) : null,
		job.repository_name ?? '',
		job.attempt > 1 ? t('jobs.meta.attempt', { n: job.attempt }) : null
	];
}

export function isUnfinished(job: Pick<Job, 'status'>): boolean {
	return job.status === 'queued' || job.status === 'sent' || job.status === 'running';
}

/** The duration counts to now, so the list item must render again on each read. */
export function isTimed(job: Pick<Job, 'started_at' | 'finished_at'>): boolean {
	return Boolean(job.started_at) && !job.finished_at;
}

/** The duration so far or in full; a dash for a finished job that never started. */
export function jobDuration(job: Job): string {
	if (job.started_at) return elapsed(secondsSpan(job.started_at, job.finished_at));
	return isUnfinished(job) ? '' : '—';
}
