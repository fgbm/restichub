/*
 * The sentence of an alert. The server stores the typed values of a condition, and the interface
 * builds the text in the language of the reader (ADR-0036). The emails and the webhook messages
 * use the same templates on the server.
 */
import { formatTime } from './format';
import { t, tOr } from './i18n';
import type { MessageKey } from './i18n/en';
import type { Alert } from './api/types';

function duration(seconds: number): string {
	const minutes = Math.floor(seconds / 60);
	return minutes >= 120
		? t('alerts.duration.hours', { n: Math.floor(minutes / 60) })
		: t('alerts.duration.minutes', { n: minutes });
}

function errorCodeText(code: string): string {
	return tOr(`jobs.errorCode.${code}`, code);
}

/** The sentence that tells why the alert exists. It holds no error text of a job. */
export function alertCondition(alert: Alert): string {
	const params = alert.params;
	const never = alert.kind === 'rpo_violated' && !params.last_success_at;
	const key = `alerts.cond.${alert.kind}${never ? '_never' : ''}` as MessageKey;
	return t(key, {
		elapsed: duration(params.elapsed_seconds ?? 0),
		rpo: duration(params.rpo_seconds ?? 0),
		threshold: params.threshold_minutes ?? 0,
		at: params.last_success_at ? formatTime(params.last_success_at) : '',
		what: t(`jobs.kind.${params.job_kind ?? 'other'}` as MessageKey).toLowerCase(),
		code: errorCodeText(params.error_code ?? 'failed')
	});
}
