import { describe, expect, it } from 'vitest';
import {
	formatBytes,
	formatDuration,
	formatMinutes,
	formatSeconds,
	formatTimeInZoneWithLocal
} from './format';

describe('formatTimeInZoneWithLocal', () => {
	it('shows the local time in the style of the zone time, without seconds', () => {
		const text = formatTimeInZoneWithLocal('2026-10-15T04:30:00Z', 'Pacific/Auckland');
		expect(text).toContain('Pacific/Auckland');
		expect(text).not.toMatch(/\d{2}:\d{2}:\d{2}/);
		expect(text).toMatch(/· .* local$/);
	});

	it('omits the local time when it equals the zone time', () => {
		const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
		const text = formatTimeInZoneWithLocal('2026-10-15T04:30:00Z', zone);
		expect(text.endsWith(zone)).toBe(true);
		expect(text).not.toContain('local');
	});
});

describe('formatDuration', () => {
	it('shows the two largest units', () => {
		expect(formatDuration('2026-10-14T10:00:00Z', '2026-10-14T11:05:30Z')).toBe('1h 5m');
		expect(formatDuration('2026-10-14T10:00:00Z', '2026-10-14T10:12:03Z')).toBe('12m 3s');
		expect(formatDuration('2026-10-14T10:00:00Z', '2026-10-14T10:00:07Z')).toBe('7s');
	});

	it('shows zero seconds for an empty span', () => {
		expect(formatDuration('2026-10-14T10:00:00Z', '2026-10-14T10:00:00Z')).toBe('0s');
	});
});

describe('formatBytes', () => {
	it('uses binary units and keeps whole bytes below 1 KiB', () => {
		expect(formatBytes(0)).toBe('0 B');
		expect(formatBytes(1023)).toBe('1,023 B');
		expect(formatBytes(1024)).toBe('1 KiB');
		expect(formatBytes(1_503_238_553)).toBe('1.4 GiB');
		expect(formatBytes(5 * 1024 ** 4)).toBe('5 TiB');
	});
});

describe('formatSeconds', () => {
	it('rounds to whole seconds and shows two units', () => {
		expect(formatSeconds(192.4)).toBe('3m 12s');
		expect(formatSeconds(0.2)).toBe('0s');
	});
});

describe('formatMinutes', () => {
	it('shows hours and minutes', () => {
		expect(formatMinutes(60)).toBe('1h');
		expect(formatMinutes(2880)).toBe('48h');
		expect(formatMinutes(90)).toBe('1h 30m');
		expect(formatMinutes(30)).toBe('30m');
	});
});
