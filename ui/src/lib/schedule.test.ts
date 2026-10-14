import { describe, expect, it } from 'vitest';
import type { MessageKey } from '#lib/i18n/index.ts';
import { describeCron, describeMissedRunPolicy } from './schedule';

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
