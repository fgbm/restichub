/*
 * Schedule helpers of the Plans screen (FR-SC-01, FR-SC-07).
 *
 * A cron expression becomes a short phrase for the common forms. An unusual
 * expression keeps its raw form, so the screen never hides the schedule. The
 * caller passes the translation function.
 */
import type { MessageKey } from '#lib/i18n/index.ts';
import type { MissedRunPolicy } from '#lib/api/index.ts';

type Translate = (key: MessageKey, params?: Record<string, string | number>) => string;

/** A phrase for a five-field cron expression, or the expression itself. */
export function describeCron(cron: string, t: Translate): string {
	const fields = cron.trim().split(/\s+/);
	if (fields.length !== 5) return cron;
	const [minute, hour, day, month, weekday] = fields;
	if (!/^\d+$/.test(minute)) return cron;
	if (hour === '*') return t('plans.cron.hourly', { minute: pad(minute) });
	if (/^\d+$/.test(hour) && day === '*' && month === '*' && weekday === '*') {
		return t('plans.cron.dailyAt', { time: `${pad(hour)}:${pad(minute)}` });
	}
	return cron;
}

/** A phrase for the missed run policy of a plan (FR-SC-07). */
export function describeMissedRunPolicy(policy: MissedRunPolicy, t: Translate): string {
	switch (policy) {
		case 'run_all':
			return t('plans.missedRunPolicy.runAll');
		case 'latest_only':
			return t('plans.missedRunPolicy.latestOnly');
		case 'skip':
			return t('plans.missedRunPolicy.skip');
	}
}

function pad(value: string): string {
	return value.padStart(2, '0');
}
