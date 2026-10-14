import { describe, expect, it } from 'vitest';
import { formatDuration, formatTimeInZoneWithLocal } from './format';

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
