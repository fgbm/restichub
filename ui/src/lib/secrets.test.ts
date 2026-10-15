import { describe, expect, it } from 'vitest';
import type { Secret } from '#lib/api/index.ts';
import { canReplaceValue, purposeKey, usageHref, wasReplaced } from './secrets';

function secret(overrides: Partial<Secret>): Secret {
	return {
		id: 's1',
		name: 'db.password',
		purpose: 'hook_secret',
		created_at: '2026-10-01T10:00:00Z',
		updated_at: '2026-10-01T10:00:00Z',
		created_by: null,
		used_by: [],
		...overrides
	};
}

describe('secrets screen rules', () => {
	it('names the known purposes only', () => {
		expect(purposeKey('hook_secret')).toBe('secrets.purpose.hook_secret');
		expect(purposeKey('webhook_url')).toBe('secrets.purpose.webhook_url');
		expect(purposeKey('unknown')).toBeNull();
	});

	it('refuses a replace of a repository password and of a webhook secret', () => {
		expect(canReplaceValue(secret({}))).toBe(true);
		expect(canReplaceValue(secret({ purpose: 'repository_password' }))).toBe(true);
		expect(
			canReplaceValue(
				secret({
					purpose: 'repository_password',
					used_by: [{ kind: 'repository', id: 'r1', name: 'main' }]
				})
			)
		).toBe(false);
		expect(canReplaceValue(secret({ purpose: 'webhook_secret' }))).toBe(false);
	});

	it('links each kind of user to its screen', () => {
		expect(usageHref({ kind: 'repository', id: 'r1', name: 'main' })).toBe('/(app)/repositories');
		expect(usageHref({ kind: 'profile', id: 'p1', name: 'nightly' })).toBe('/(app)/profiles');
		expect(usageHref({ kind: 'channel', id: 'c1', name: 'mail' })).toBe(
			'/(app)/settings/notifications/channels'
		);
	});

	it('detects a replaced value', () => {
		expect(wasReplaced(secret({}))).toBe(false);
		expect(wasReplaced(secret({ updated_at: '2026-10-02T10:00:00Z' }))).toBe(true);
	});
});
