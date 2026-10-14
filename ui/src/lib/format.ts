import { getLocale } from '#lib/i18n/index.ts';

export function formatTime(value: string): string {
	return new Date(value).toLocaleString(getLocale());
}

/** Formats an instant in a given IANA time zone (FR-SC-01). */
export function formatTimeInZone(value: string, timeZone: string): string {
	try {
		return new Intl.DateTimeFormat(getLocale(), {
			dateStyle: 'medium',
			timeStyle: 'short',
			timeZone
		}).format(new Date(value));
	} catch {
		return formatTime(value);
	}
}

const relativeSteps: [Intl.RelativeTimeFormatUnit, number][] = [
	['second', 60],
	['minute', 60],
	['hour', 24],
	['day', 30],
	['month', 12],
	['year', Infinity]
];

/** A short distance to now, such as "14 minutes ago", in the current locale. */
export function relativeTime(value: string): string {
	let amount = (new Date(value).getTime() - Date.now()) / 1000;
	const format = new Intl.RelativeTimeFormat(getLocale(), { numeric: 'auto' });
	for (const [unit, size] of relativeSteps) {
		if (Math.abs(amount) < size) return format.format(Math.round(amount), unit);
		amount /= size;
	}
	return formatTime(value);
}
