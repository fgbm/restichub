/*
 * The reason line of a finished job. The server stores a stable code and, for an incompatible job,
 * the typed values (AC-AG-09). The interface builds the text in the language of the reader, so the
 * raw code never reaches the screen (NFR-DOC-04).
 */
import { t, tOr } from './i18n';
import type { Job } from './api/types';

type ReasonSource = Pick<Job, 'result_message' | 'result_code' | 'result_params'>;

export function jobReason(job: ReasonSource): string | null {
	const code =
		job.result_message ?? (job.result_code === 'repository_locked' ? job.result_code : null);
	if (!code) return null;
	const params = job.result_params;
	if (code === 'incompatible' && params) {
		return params.option
			? t('jobs.reason.incompatibleOption', {
					option: params.option,
					required: params.required_version,
					installed: params.installed_version
				})
			: t('jobs.reason.incompatibleVersion', {
					required: params.required_version,
					installed: params.installed_version
				});
	}
	return tOr(`jobs.reason.${code}`, code);
}
