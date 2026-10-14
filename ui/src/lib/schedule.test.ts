import { describe, expect, it } from 'vitest';
import type { MessageKey } from '#lib/i18n/index.ts';
import {
	browserTimeZone,
	describeCron,
	describeMissedRunPolicy,
	nextRunsFromCron,
	timeZoneNames
} from './schedule';

const t = (key: MessageKey): string => key;

describe('the schedule helpers', () => {
	it('describes a daily cron expression', () => {
		expect(describeCron('0 2 * * *', t)).toBe('plans.cron.dailyAt');
	});

	it('describes an hourly cron expression', () => {
		expect(describeCron('15 * * * *', t)).toBe('plans.cron.hourly');
	});

	it('keeps an unusual expression', () => {
		expect(describeCron('*/5 3 * * 1-5', t)).toBe('*/5 3 * * 1-5');
		expect(describeCron('0 2 * *', t)).toBe('0 2 * *');
	});

	it('names every missed run policy', () => {
		expect(describeMissedRunPolicy('run_all', t)).toBe('plans.missedRunPolicy.runAll');
		expect(describeMissedRunPolicy('latest_only', t)).toBe('plans.missedRunPolicy.latestOnly');
		expect(describeMissedRunPolicy('skip', t)).toBe('plans.missedRunPolicy.skip');
	});
});

describe('the cron preview of the mock', () => {
	it('computes the next hourly runs from the cron, not a fixed interval', () => {
		// The runs are on the hour, one hour apart, from a known instant.
		const from = new Date('2026-01-01T00:00:30Z');
		const runs = nextRunsFromCron('0 * * * *', 'UTC', 3, from);
		expect(runs).toEqual([
			'2026-01-01T01:00:00.000Z',
			'2026-01-01T02:00:00.000Z',
			'2026-01-01T03:00:00.000Z'
		]);
	});

	it('computes a daily run at the local wall clock of the zone', () => {
		const from = new Date('2026-01-01T00:00:00Z');
		const runs = nextRunsFromCron('0 2 * * *', 'Europe/Berlin', 2, from);
		// 02:00 CET is 01:00 UTC.
		expect(runs).toEqual(['2026-01-01T01:00:00.000Z', '2026-01-02T01:00:00.000Z']);
	});

	it('computes a fifteen-minute step', () => {
		const from = new Date('2026-01-01T00:01:00Z');
		const runs = nextRunsFromCron('*/15 * * * *', 'UTC', 3, from);
		expect(runs).toEqual([
			'2026-01-01T00:15:00.000Z',
			'2026-01-01T00:30:00.000Z',
			'2026-01-01T00:45:00.000Z'
		]);
	});

	it('refuses a malformed expression', () => {
		expect(nextRunsFromCron('0 2 * *', 'UTC', 3)).toEqual([]);
	});
});

describe('the IANA time zone list', () => {
	it('holds a browser zone', () => {
		expect(timeZoneNames().length).toBeGreaterThan(0);
		expect(typeof browserTimeZone()).toBe('string');
	});
});
