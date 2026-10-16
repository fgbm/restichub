import { afterEach, describe, expect, it } from 'vitest';
import { setLocale } from '#lib/i18n/index.ts';
import {
	ago,
	changedAt,
	clockTime,
	dayHeading,
	dayTime,
	elapsed,
	expiry,
	inFuture,
	plural,
	size
} from './list-format';

const now = new Date(2026, 9, 16, 9, 41, 0).getTime();
const local = (...parts: number[]) =>
	new Date(...(parts as [number, number, number, number, number, number?])).toISOString();

afterEach(() => setLocale('en'));

describe('list time formats', () => {
	it('shows freshness as a short distance', () => {
		expect(ago(new Date(now - 24_000).toISOString(), now)).toBe('24 s ago');
		expect(ago(new Date(now - 7 * 60_000).toISOString(), now)).toBe('7 min ago');
		expect(ago(new Date(now - 3 * 3_600_000).toISOString(), now)).toBe('3 h ago');
		expect(ago(new Date(now - 2 * 86_400_000).toISOString(), now)).toBe('2 d ago');
		setLocale('ru');
		expect(ago(new Date(now - 2 * 86_400_000).toISOString(), now)).toBe('2 дн назад');
		expect(inFuture(new Date(now + 23 * 3_600_000).toISOString(), now)).toBe('через 23 ч');
	});

	it('shows a run with the day and the hours', () => {
		setLocale('ru');
		expect(dayTime(local(2026, 9, 16, 2, 0), now)).toBe('сегодня 02:00');
		expect(dayTime(local(2026, 9, 15, 23, 30), now)).toBe('вчера 23:30');
		expect(dayTime(local(2026, 9, 14, 2, 0), now)).toBe('14 окт 02:00');
		expect(dayTime(local(2025, 9, 14, 2, 0), now)).toBe('14 окт 2025');
	});

	it('shows a configuration change as relative up to 7 days, then as a date', () => {
		setLocale('ru');
		expect(changedAt(new Date(now - 2 * 3_600_000).toISOString(), now)).toBe('2 ч назад');
		expect(changedAt(local(2026, 8, 3, 12, 0), now)).toBe('3 сен');
	});

	it('shows a validity end as a date, or as a distance when less than 30 days remain', () => {
		setLocale('ru');
		expect(expiry(new Date(now + 9 * 86_400_000).toISOString(), now)).toEqual({
			text: 'истекает через 9 дн',
			soon: true
		});
		expect(expiry(local(2027, 3, 12, 12, 0), now)).toEqual({ text: 'до 12.04.2027', soon: false });
	});

	it('names the day of a group and keeps seconds in the audit time', () => {
		setLocale('ru');
		expect(dayHeading(local(2026, 9, 16, 9, 40), now)).toBe('Сегодня, 16 октября');
		expect(dayHeading(local(2026, 9, 15, 9, 40), now)).toBe('Вчера, 15 октября');
		expect(dayHeading(local(2026, 9, 3, 9, 40), now)).toBe('3 октября');
		expect(clockTime(local(2026, 9, 16, 9, 40, 2))).toBe('09:40:02');
	});
});

describe('list number formats', () => {
	it('shows a duration as seconds, minutes and seconds, or hours and minutes', () => {
		setLocale('ru');
		expect(elapsed(42)).toBe('42 с');
		expect(elapsed(252)).toBe('4 мин 12 с');
		expect(elapsed(3840)).toBe('1 ч 04 мин');
	});

	it('shows a size in binary units with 3 significant digits', () => {
		setLocale('ru');
		expect(size(412 * 1024)).toBe('412 КиБ');
		expect(size(12.4 * 1024 ** 3)).toBe('12,4 ГиБ');
		expect(size(1.02 * 1024 ** 4)).toBe('1,02 ТиБ');
		setLocale('en');
		expect(size(512)).toBe('512 B');
	});

	it('picks the plural form of the locale', () => {
		setLocale('ru');
		expect(plural('list.scripts', 1)).toBe('1 скрипт');
		expect(plural('list.scripts', 3)).toBe('3 скрипта');
		expect(plural('list.scripts', 5)).toBe('5 скриптов');
		setLocale('en');
		expect(plural('list.scripts', 1)).toBe('1 script');
		expect(plural('list.scripts', 5)).toBe('5 scripts');
	});
});
