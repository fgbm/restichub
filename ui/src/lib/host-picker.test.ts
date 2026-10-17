import { describe, expect, it } from 'vitest';
import { hostOptions, hostPresence, withHosts, type HostRef } from './host-picker';

const NOW = Date.parse('2026-10-17T12:00:00Z');

function host(overrides: Partial<HostRef> = {}): HostRef {
	return {
		id: 'a_1',
		hostname: 'fs-01',
		os: 'windows',
		arch: 'amd64',
		labels: ['prod'],
		status: 'active',
		online: true,
		first_connected_at: '2026-10-01T00:00:00Z',
		last_seen_at: '2026-10-17T11:59:00Z',
		...overrides
	};
}

describe('the host picker options', () => {
	it('names the state in words next to the dot', () => {
		expect(hostPresence(host(), NOW)).toEqual({ dot: 'success', label: 'Online' });
		expect(
			hostPresence(host({ online: false, last_seen_at: '2026-10-17T09:00:00Z' }), NOW)
		).toEqual({ dot: 'neutral', label: 'Offline · 3 h ago' });
		expect(hostPresence(host({ first_connected_at: null, online: false }), NOW).dot).toBe(
			'warning'
		);
		expect(hostPresence(host({ status: 'revoked' }), NOW).label).toBe('Revoked');
	});

	it('shows the platform and the labels, and the OS family and the state when closed', () => {
		const [option] = hostOptions([host()], undefined, NOW);
		expect(option.description).toEqual(['Windows · amd64', { tag: 'prod' }]);
		expect(option.detail).toBe('Windows · Online');
		expect(option.disabled).toBe(false);
		expect(hostOptions([host({ os: '' })], undefined, NOW)[0].description?.[0]).toBe(
			'OS not known yet'
		);
	});

	it('disables a host with a reason and puts the revoked hosts last', () => {
		const options = hostOptions(
			[host({ id: 'a_0', hostname: 'old', status: 'revoked' }), host(), host({ id: 'a_2' })],
			(item) => (item.id === 'a_2' ? 'Repository local-fs is on fs-01' : null),
			NOW
		);
		expect(options.map((option) => option.value)).toEqual(['a_1', 'a_2', 'a_0']);
		expect(options[1]).toMatchObject({ disabled: true, reason: 'Repository local-fs is on fs-01' });
	});

	it('gives the facet values the host line and says that a host is revoked', () => {
		const hosts = [host({ id: 'a_0', hostname: 'old', status: 'revoked', labels: [] }), host()];
		const options = withHosts(
			[{ value: 'a_0', count: 1 }, { value: 'a_1' }, { value: 'x' }],
			hosts
		);
		expect(options).toEqual([
			{ value: 'a_1', label: 'fs-01', description: ['Windows · amd64', { tag: 'prod' }] },
			{ value: 'x' },
			{ value: 'a_0', count: 1, label: 'old', description: ['Windows · amd64', 'Revoked'] }
		]);
	});
});
