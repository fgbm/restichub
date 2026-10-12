/*
 * Tests for the HTTP service layer. The tests replace `fetch` and read the
 * request that the service sends and the body that it returns.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ApiService } from './service';

function jsonResponse(body: unknown, status = 200): Response {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json' }
	});
}

/** Loads the service with a replaced `fetch`. */
async function loadService(fetchFn: ReturnType<typeof vi.fn>): Promise<ApiService> {
	vi.resetModules();
	vi.stubGlobal('fetch', fetchFn);
	return (await import('./service')).service;
}

afterEach(() => {
	vi.unstubAllGlobals();
});

describe('the HTTP service (issue #20 tenants)', () => {
	it('unwraps the tenant list items', async () => {
		const tenant = {
			id: 't_01',
			name: 'acme',
			status: 'active',
			created_at: '2026-10-12T10:00:00Z',
			user_count: 3
		};
		const fetchFn = vi.fn().mockResolvedValue(jsonResponse({ items: [tenant], next_cursor: null }));
		const service = await loadService(fetchFn);

		const tenants = await service.listTenants();
		expect(tenants).toEqual([tenant]);
		expect(fetchFn.mock.calls[0][0]).toContain('/api/v1/platform/tenants');
	});

	it('posts the create-tenant body and returns the tenant', async () => {
		const tenant = {
			id: 't_02',
			name: 'globex',
			status: 'active',
			created_at: '2026-10-12T10:00:00Z',
			user_count: 1
		};
		const fetchFn = vi.fn().mockResolvedValue(jsonResponse(tenant, 201));
		const service = await loadService(fetchFn);

		const created = await service.createTenant({
			name: 'globex',
			first_admin: { username: 'admin', display_name: 'Admin', password: 'a-long-password' }
		});
		expect(created).toEqual(tenant);
		const init = fetchFn.mock.calls[0][1] as RequestInit;
		expect(init.method).toBe('POST');
		expect(JSON.parse(init.body as string)).toEqual({
			name: 'globex',
			first_admin: { username: 'admin', display_name: 'Admin', password: 'a-long-password' }
		});
	});

	it('suspend, resume and delete return no body', async () => {
		const fetchFn = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
		const service = await loadService(fetchFn);

		await expect(service.suspendTenant('t_01')).resolves.toBeUndefined();
		await expect(service.resumeTenant('t_01')).resolves.toBeUndefined();
		await expect(service.deleteTenant('t_01')).resolves.toBeUndefined();

		expect(fetchFn.mock.calls[0][0]).toContain('/api/v1/platform/tenants/t_01/suspend');
		expect(fetchFn.mock.calls[1][0]).toContain('/api/v1/platform/tenants/t_01/resume');
		expect(fetchFn.mock.calls[2][0]).toContain('/api/v1/platform/tenants/t_01');
		expect((fetchFn.mock.calls[2][1] as RequestInit).method).toBe('DELETE');
	});

	it('unwraps the user list items', async () => {
		const user = {
			id: 'u_01',
			username: 'admin',
			display_name: 'Admin',
			disabled: false,
			role: 'admin'
		};
		const fetchFn = vi.fn().mockResolvedValue(jsonResponse({ items: [user] }));
		const service = await loadService(fetchFn);

		expect(await service.listUsers()).toEqual([user]);
		expect(fetchFn.mock.calls[0][0]).toContain('/api/v1/users');
	});
});
