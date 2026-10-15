/*
 * Tests for the in-memory mock of the enrollment tokens. A screen and an e2e test use the
 * mock, so its lifecycle follows the control plane: the default TTL is one hour (ADR-0032),
 * the list holds the token until the revoke, and the second revoke is a conflict.
 */
import { afterEach, describe, expect, it } from 'vitest';
import { MockApiService } from './mock';

afterEach(() => {
	localStorage.removeItem('restichub.mock.agents');
});

describe('the mock enrollment tokens', () => {
	it('creates a token with the one-hour default and revokes it once', async () => {
		const api = new MockApiService();
		const created = await api.createEnrollmentToken({ labels: ['web'] });
		const ttl = new Date(created.expires_at).getTime() - new Date(created.created_at).getTime();
		expect(ttl).toBe(60 * 60 * 1000);

		const listed = await api.listEnrollmentTokens();
		expect(listed.map((token) => token.id)).toContain(created.id);

		await api.revokeEnrollmentToken(created.id);
		const revoked = (await api.listEnrollmentTokens()).find((token) => token.id === created.id);
		expect(revoked?.revoked_at).not.toBeNull();

		await expect(api.revokeEnrollmentToken(created.id)).rejects.toMatchObject({ code: 'token_already_revoked' });
	});
});

describe('the mock agents', () => {
	it('renames an agent', async () => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		const api = new MockApiService();
		const [first] = await api.listAgents();

		const renamed = await api.renameAgent(first.id, 'win-fs01-renamed');
		expect(renamed.hostname).toBe('win-fs01-renamed');
		const listed = (await api.listAgents()).find((agent) => agent.id === first.id);
		expect(listed?.hostname).toBe('win-fs01-renamed');
	});
});
