import { getLocale, t } from '#lib/i18n/index.ts';

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

/** Formats an instant in the time zone of the browser, in the style of [`formatTimeInZone`]. */
export function formatLocalTime(value: string): string {
	return new Intl.DateTimeFormat(getLocale(), { dateStyle: 'medium', timeStyle: 'short' }).format(
		new Date(value)
	);
}

/**
 * Formats an instant in a given IANA time zone and appends the zone name; the local time follows
 * when it differs (FR-SC-01).
 */
export function formatTimeInZoneWithLocal(value: string, timeZone: string): string {
	const zoned = formatTimeInZone(value, timeZone);
	const local = formatLocalTime(value);
	return local === zoned
		? `${zoned} ${timeZone}`
		: `${zoned} ${timeZone} · ${local} ${t('common.localTime')}`;
}

/** The distance between two instants as hours, minutes and seconds, such as "1 h 5 min". */
export function formatDuration(from: string, to: string): string {
	const total = Math.max(0, Math.round((new Date(to).getTime() - new Date(from).getTime()) / 1000));
	const parts: [number, string][] = [
		[Math.floor(total / 3600), 'hour'],
		[Math.floor((total % 3600) / 60), 'minute'],
		[total % 60, 'second']
	];
	// Show the two largest non-zero units; a zero duration shows "0 s".
	const shown = parts.filter(([amount]) => amount > 0).slice(0, 2);
	if (shown.length === 0) shown.push([0, 'second']);
	return shown
		.map(([amount, unit]) =>
			new Intl.NumberFormat(getLocale(), {
				style: 'unit',
				unit,
				unitDisplay: 'narrow'
			}).format(amount)
		)
		.join(' ');
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

const byteUnits = ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB'];

/** A size in binary units, such as "1.4 GiB". Under 1 KiB it shows whole bytes. */
export function formatBytes(value: number): string {
	let amount = Math.max(0, value);
	let unit = 0;
	while (amount >= 1024 && unit < byteUnits.length - 1) {
		amount /= 1024;
		unit += 1;
	}
	const format = new Intl.NumberFormat(getLocale(), {
		maximumFractionDigits: unit === 0 ? 0 : amount < 10 ? 2 : 1
	});
	return `${format.format(amount)} ${byteUnits[unit]}`;
}

/** An integer with the digit grouping of the locale. */
export function formatCount(value: number): string {
	return new Intl.NumberFormat(getLocale()).format(value);
}

/** A time of day with seconds, in the time zone of the browser, such as "21:11:02". */
export function formatClock(value: string): string {
	return new Intl.DateTimeFormat(getLocale(), { timeStyle: 'medium', hour12: false }).format(
		new Date(value)
	);
}

/** A whole number of minutes as hours and minutes, such as "1h 30m". */
export function formatMinutes(minutes: number): string {
	return formatSeconds(minutes * 60);
}

/** Seconds as hours, minutes and seconds, such as "2 min 41 s". */
export function formatSeconds(seconds: number): string {
	const base = new Date(0).toISOString();
	return formatDuration(base, new Date(Math.max(0, Math.round(seconds)) * 1000).toISOString());
}
