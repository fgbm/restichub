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

/** The fallback IANA names when `Intl.supportedValuesOf` is absent. */
export const FALLBACK_TIME_ZONES = [
	'UTC',
	'Europe/London',
	'Europe/Berlin',
	'Europe/Paris',
	'Europe/Moscow',
	'Asia/Dubai',
	'Asia/Kolkata',
	'Asia/Shanghai',
	'Asia/Tokyo',
	'America/New_York',
	'America/Chicago',
	'America/Denver',
	'America/Los_Angeles',
	'Australia/Sydney'
];

/** The IANA names of the browser, or the fallback list (FR-SC-01). */
export function timeZoneNames(): string[] {
	const supported = (
		Intl as unknown as { supportedValuesOf?: (key: string) => string[] }
	).supportedValuesOf?.('timeZone');
	return Array.isArray(supported) && supported.length > 0 ? supported : FALLBACK_TIME_ZONES;
}

/** The time zone of the browser, or `UTC` (FR-SC-01). */
export function browserTimeZone(): string {
	try {
		return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
	} catch {
		return 'UTC';
	}
}

type ZoneParts = { minute: number; hour: number; day: number; month: number; weekday: number };

const PART_FORMATTERS = new Map<string, Intl.DateTimeFormat>();

function zoneParts(at: Date, timeZone: string): ZoneParts {
	let formatter = PART_FORMATTERS.get(timeZone);
	if (!formatter) {
		formatter = new Intl.DateTimeFormat('en-US', {
			timeZone,
			hour12: false,
			hour: '2-digit',
			minute: '2-digit',
			day: '2-digit',
			month: '2-digit',
			weekday: 'short'
		});
		PART_FORMATTERS.set(timeZone, formatter);
	}
	const parts = formatter.formatToParts(at);
	const get = (type: string) => parts.find((part) => part.type === type)?.value ?? '';
	const weekdayNames = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
	return {
		minute: Number(get('minute')),
		hour: Number(get('hour')) % 24,
		day: Number(get('day')),
		month: Number(get('month')),
		weekday: Math.max(0, weekdayNames.indexOf(get('weekday')))
	};
}

/** A cron field holds `*`, a number, a list, a range or a step. `max` is the largest value. */
function fieldMatches(field: string, value: number, max: number): boolean {
	return field.split(',').some((part) => {
		const [range, stepText] = part.split('/');
		const step = stepText ? Number(stepText) : 1;
		if (!Number.isFinite(step) || step < 1) return false;
		if (range === '*') return value % step === 0;
		const [startText, endText] = range.split('-');
		const start = Number(startText);
		const end = endText === undefined ? (stepText ? max : start) : Number(endText);
		if (!Number.isFinite(start) || !Number.isFinite(end)) return false;
		if (value < start || value > end) return false;
		return (value - start) % step === 0;
	});
}

/**
 * The next fire times of a five-field cron expression in a time zone, as ISO instants (FR-SC-01).
 *
 * The mock screen has no server, so it computes the runs the way the control plane does. The walk
 * runs minute by minute, from the minute after `from`, and stops at the count or after five years.
 * A seven in the day-of-week field is the Sunday that a cron of zero names.
 */
export function nextRunsFromCron(
	cron: string,
	timeZone: string,
	count: number,
	from: Date = new Date()
): string[] {
	const fields = cron.trim().split(/\s+/);
	if (fields.length !== 5 || count <= 0) return [];
	const [minute, hour, day, month, weekday] = fields;
	const runs: string[] = [];
	const step = 60_000;
	let cursor = Math.floor(from.getTime() / step) * step + step;
	const limit = cursor + 5 * 366 * 24 * 60 * step;
	while (cursor <= limit && runs.length < count) {
		const at = new Date(cursor);
		const parts = zoneParts(at, timeZone);
		const weekdayValue = parts.weekday === 0 ? 7 : parts.weekday;
		if (
			fieldMatches(minute, parts.minute, 59) &&
			fieldMatches(hour, parts.hour, 23) &&
			fieldMatches(day, parts.day, 31) &&
			fieldMatches(month, parts.month, 12) &&
			(fieldMatches(weekday, parts.weekday, 7) || fieldMatches(weekday, weekdayValue, 7))
		) {
			runs.push(at.toISOString());
		}
		cursor += step;
	}
	return runs;
}
