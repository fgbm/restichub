/*
 * The reason line of a finished job. The server stores a stable code and, for an incompatible job,
 * the typed values (AC-AG-09). The interface builds the text in the language of the reader, so the
 * raw code never reaches the screen (NFR-DOC-04).
 */
import { t, tOr } from './i18n';
import type { Job, JobStep } from './api/types';

type ReasonSource = Pick<Job, 'result_message' | 'result_code' | 'result_params'>;
type StepSource = Pick<JobStep, 'error_code' | 'error_params'>;

export function jobReason(job: ReasonSource): string | null {
	const code =
		job.result_message ?? (job.result_code === 'repository_locked' ? job.result_code : null);
	if (!code) return null;
	const params = job.result_params;
	if (code === 'incompatible' && params) {
		return params.option
			? t('jobs.incompatible.option', {
					option: params.option,
					required: params.required_version,
					installed: params.installed_version
				})
			: t('jobs.incompatible.version', {
					required: params.required_version,
					installed: params.installed_version
				});
	}
	return tOr(`jobs.reason.${code}`, code);
}

/*
 * The cause that the agent sent for a step whose restic did not start. It arrives with the step
 * result, also when no log line of the step reached the server.
 */
export function notStartedReason(step: StepSource | null | undefined): string | null {
	if (step?.error_code !== 'step_not_started') return null;
	return step.error_params?.reason?.trim() || null;
}

/* The reason line of a job page, which knows the steps: a step that did not start names its cause. */
export function jobDetailReason(job: ReasonSource & { steps: StepSource[] }): string | null {
	if (job.result_message === 'step_not_started') {
		const reason = job.steps.map(notStartedReason).find((cause) => cause !== null);
		if (reason) return t('jobs.reason.notStartedBecause', { reason });
	}
	return jobReason(job);
}
