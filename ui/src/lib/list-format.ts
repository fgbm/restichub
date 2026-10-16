/*
 * The time, size and count formats of the list items (design/style/ui-principles.md, 4).
 *
 * Each function takes `now` so that a test and a list render use one instant for all items.
 */
import { getLocale, t } from '#lib/i18n/index.ts';
import type { MessageKey } from '#lib/i18n/index.ts';
import { dateTimeFormat, numberFormat, pluralRules } from '#lib/intl.ts';

const MINUTE = 60;
const HOUR = 3600;
const DAY = 86400;

type Unit = 's' | 'min' | 'h' | 'd' | 'mo' | 'y';

function unitOf(seconds: number): [number, Unit] {
	if (seconds < MINUTE) return [Math.max(1, Math.floor(seconds)), 's'];
	if (seconds < HOUR) return [Math.floor(seconds / MINUTE), 'min'];
	if (seconds < DAY) return [Math.floor(seconds / HOUR), 'h'];
	if (seconds < 30 * DAY) return [Math.floor(seconds / DAY), 'd'];
	if (seconds < 365 * DAY) return [Math.floor(seconds / (30 * DAY)), 'mo'];
	return [Math.floor(seconds / (365 * DAY)), 'y'];
}

function secondsBetween(value: string, now: number): number {
	return (new Date(value).getTime() - now) / 1000;
}

/** A short distance to the past, such as "7 min ago". */
export function ago(value: string, now = Date.now()): string {
	const [count, unit] = unitOf(Math.max(0, -secondsBetween(value, now)));
	return t(`time.ago.${unit}` as MessageKey, { n: count });
}

/** A short distance to the future, such as "in 23 h". */
export function inFuture(value: string, now = Date.now()): string {
	const [count, unit] = unitOf(Math.max(0, secondsBetween(value, now)));
	return t(`time.in.${unit}` as MessageKey, { n: count });
}

function startOfDay(time: number): number {
	const date = new Date(time);
	date.setHours(0, 0, 0, 0);
	return date.getTime();
}

function hoursMinutes(date: Date): string {
	return dateTimeFormat({ hour: '2-digit', minute: '2-digit', hour12: false }).format(date);
}

// A three-letter month without a dot, as in "14 Oct" and "3 сен".
function dayMonth(date: Date, withYear: boolean): string {
	const month = t('time.months').split(',')[date.getMonth()];
	const params = { day: date.getDate(), month, year: date.getFullYear() };
	return withYear ? t('time.dayMonthYear', params) : t('time.dayMonth', params);
}

/** The day and the hours of a run: "today 02:00", "yesterday 23:30", "14 Oct 02:00", "14 Oct 2025". */
export function dayTime(value: string, now = Date.now()): string {
	const date = new Date(value);
	const day = startOfDay(date.getTime());
	const today = startOfDay(now);
	if (day === today) return t('time.today', { time: hoursMinutes(date) });
	if (day === startOfDay(today - 1)) return t('time.yesterday', { time: hoursMinutes(date) });
	if (date.getFullYear() !== new Date(now).getFullYear()) return dayMonth(date, true);
	return `${dayMonth(date, false)} ${hoursMinutes(date)}`;
}

/** A configuration change: relative up to 7 days, then the date, such as "3 Sep". */
export function changedAt(value: string, now = Date.now()): string {
	if (-secondsBetween(value, now) < 7 * DAY) return ago(value, now);
	const date = new Date(value);
	return dayMonth(date, date.getFullYear() !== new Date(now).getFullYear());
}

/** A validity end: the date, or the distance when less than 30 days remain. */
export function expiry(value: string, now = Date.now()): { text: string; soon: boolean } {
	const left = secondsBetween(value, now);
	if (left < 30 * DAY)
		return { text: t('time.expiresIn', { time: inFuture(value, now) }), soon: true };
	return {
		text: t('time.validUntil', { date: new Date(value).toLocaleDateString(getLocale()) }),
		soon: false
	};
}

/** The heading of a day group: "Today, 16 October", "Yesterday, 15 October", "14 October 2025". */
export function dayHeading(value: string, now = Date.now()): string {
	const date = new Date(value);
	const day = startOfDay(date.getTime());
	const today = startOfDay(now);
	const sameYear = date.getFullYear() === new Date(now).getFullYear();
	const label = dateTimeFormat({
		day: 'numeric',
		month: 'long',
		...(sameYear ? {} : { year: 'numeric' })
	}).format(date);
	if (day === today) return t('time.todayHeading', { date: label });
	if (day === startOfDay(today - 1)) return t('time.yesterdayHeading', { date: label });
	return label;
}

/** The key of the local day of an instant, for the day groups of a list. */
export function dayKey(value: string): string {
	const date = new Date(value);
	return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
}

/** The time of day with seconds, such as "09:40:02". */
export function clockTime(value: string): string {
	return dateTimeFormat({
		hour: '2-digit',
		minute: '2-digit',
		second: '2-digit',
		hour12: false
	}).format(new Date(value));
}

/** The tooltip of each time: the full date, the time with seconds and the time zone. */
export function fullTime(value: string): string {
	return dateTimeFormat({
		day: 'numeric',
		month: 'long',
		year: 'numeric',
		hour: '2-digit',
		minute: '2-digit',
		second: '2-digit',
		hour12: false,
		timeZoneName: 'short'
	}).format(new Date(value));
}

const pad = (value: number) => String(value).padStart(2, '0');

/** A duration: "42 s", "4 min 12 s", "1 h 04 min". */
export function elapsed(seconds: number): string {
	const total = Math.max(0, Math.round(seconds));
	if (total < MINUTE) return t('duration.s', { s: total });
	if (total < HOUR) {
		return t('duration.ms', { m: Math.floor(total / MINUTE), s: pad(total % MINUTE) });
	}
	return t('duration.hm', {
		h: Math.floor(total / HOUR),
		m: pad(Math.floor((total % HOUR) / MINUTE))
	});
}

/** The seconds between two instants; an open end counts to `now`. */
export function secondsSpan(from: string, to: string | null | undefined, now = Date.now()): number {
	const end = to ? new Date(to).getTime() : now;
	return Math.max(0, (end - new Date(from).getTime()) / 1000);
}

const SIZE_UNITS = ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB'] as const;

/** A size in binary units with 3 significant digits: "412 KiB", "12.4 GiB", "1.02 TiB". */
export function size(bytes: number): string {
	let amount = Math.max(0, bytes);
	let unit = 0;
	while (amount >= 1000 && unit < SIZE_UNITS.length - 1) {
		amount /= 1024;
		unit += 1;
	}
	const number = numberFormat({ maximumSignificantDigits: unit === 0 ? 21 : 3 }).format(amount);
	return t(`size.${SIZE_UNITS[unit]}` as MessageKey, { n: number });
}

/** A count with the plural form of the locale; `key` has the `one`, `few`, `many` and `other` forms. */
export function plural(
	key: string,
	count: number,
	params: Record<string, string | number> = {}
): string {
	const form = pluralRules().select(count);
	return t(`${key}.${form}` as MessageKey, {
		n: numberFormat().format(count),
		...params
	});
}
