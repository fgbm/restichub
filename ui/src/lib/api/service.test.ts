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
			user_count: 3,
			admin_state: 'present'
		};
		const fetchFn = vi.fn().mockResolvedValue(jsonResponse({ items: [tenant], next_cursor: null }));
		const service = await loadService(fetchFn);

		const tenants = await service.listTenants();
		expect(tenants).toEqual([tenant]);
		expect(fetchFn.mock.calls[0][0]).toContain('/api/v1/platform/tenants');
	});

	it('reads every page of a cursor list for a screen that pages on the client', async () => {
		const fetchFn = vi
			.fn()
			.mockResolvedValueOnce(jsonResponse({ items: [{ id: 'p1' }], next_cursor: 'c1' }))
			.mockResolvedValueOnce(jsonResponse({ items: [{ id: 'p2' }], next_cursor: null }));
		const service = await loadService(fetchFn);

		expect(await service.listPlans()).toEqual([{ id: 'p1' }, { id: 'p2' }]);
		expect(fetchFn).toHaveBeenCalledTimes(2);
		expect(fetchFn.mock.calls[0][0]).toContain('limit=200');
		expect(fetchFn.mock.calls[0][0]).not.toContain('cursor=');
		expect(fetchFn.mock.calls[1][0]).toContain('cursor=c1');
	});

	it('stops with an error when the server repeats a cursor', async () => {
		const fetchFn = vi
			.fn()
			.mockImplementation(async () => jsonResponse({ items: [{ id: 'p1' }], next_cursor: 'c1' }));
		const service = await loadService(fetchFn);

		await expect(service.listPlans()).rejects.toThrow(/cursor c1 again/);
		expect(fetchFn).toHaveBeenCalledTimes(2);
	});

	it('stops with an error after the page limit', async () => {
		let page = 0;
		const fetchFn = vi.fn().mockImplementation(async () => {
			page += 1;
			return jsonResponse({ items: [{ id: `p${page}` }], next_cursor: `c${page}` });
		});
		const service = await loadService(fetchFn);

		await expect(service.listPlans()).rejects.toThrow(/more than 50 pages/);
		expect(fetchFn).toHaveBeenCalledTimes(50);
	});

	it('posts the create-tenant body and returns the tenant', async () => {
		const tenant = {
			id: 't_02',
			name: 'globex',
			status: 'active',
			created_at: '2026-10-12T10:00:00Z',
			user_count: 0,
			admin_state: 'none'
		};
		const fetchFn = vi.fn().mockResolvedValue(jsonResponse(tenant, 201));
		const service = await loadService(fetchFn);

		const created = await service.createTenant({ name: 'globex' });
		expect(created).toEqual(tenant);
		const init = fetchFn.mock.calls[0][1] as RequestInit;
		expect(init.method).toBe('POST');
		expect(JSON.parse(init.body as string)).toEqual({ name: 'globex' });
	});

	it('unwraps the platform account list items', async () => {
		const account = {
			id: 'pu_01',
			username: 'anna',
			display_name: 'Anna',
			email: null,
			email_verified: false,
			disabled: false,
			platform_admin: false,
			memberships: [{ tenant_id: 't_01', tenant_name: 'acme', role: 'admin', disabled: false }]
		};
		const fetchFn = vi.fn().mockResolvedValue(jsonResponse({ items: [account] }));
		const service = await loadService(fetchFn);

		expect(await service.listPlatformUsers()).toEqual([account]);
		expect(fetchFn.mock.calls[0][0]).toContain('/api/v1/platform/users');
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

describe('the HTTP service (unlock all)', () => {
	it('posts the repository name as the confirmation', async () => {
		const fetchFn = vi.fn().mockResolvedValue(jsonResponse({ id: 'j_1', kind: 'unlock_all' }, 202));
		const service = await loadService(fetchFn);

		await service.unlockAllRepository('r_1', 'repo-a');

		expect(fetchFn.mock.calls[0][0]).toContain('/api/v1/repositories/r_1/unlock-all');
		const request = fetchFn.mock.calls[0][1] as RequestInit;
		expect(request.method).toBe('POST');
		expect(JSON.parse(request.body as string)).toEqual({ confirm: 'repo-a' });
	});
});

describe('the HTTP service (repository maintenance)', () => {
	it('reads, replaces and runs the maintenance of a repository', async () => {
		const view = { repository_id: 'r_1', maintenance_agent_id: 'a_1' };
		const fetchFn = vi
			.fn()
			.mockResolvedValueOnce(jsonResponse(view))
			.mockResolvedValueOnce(jsonResponse(view))
			.mockResolvedValueOnce(jsonResponse({ id: 'j_1', kind: 'check' }, 202));
		const service = await loadService(fetchFn);

		await service.getMaintenance('r_1');
		await service.updateMaintenance('r_1', {
			time_zone: 'UTC',
			retention: { cron: '0 3 * * 0', enabled: true },
			check: { cron: '0 4 1 * *', enabled: false }
		});
		await service.runMaintenance('r_1', 'check');

		expect(fetchFn.mock.calls[0][0]).toContain('/api/v1/repositories/r_1/maintenance');
		const update = fetchFn.mock.calls[1][1] as RequestInit;
		expect(update.method).toBe('PUT');
		expect(JSON.parse(update.body as string).check.enabled).toBe(false);
		expect(fetchFn.mock.calls[2][0]).toContain('/api/v1/repositories/r_1/maintenance/run');
		expect(JSON.parse((fetchFn.mock.calls[2][1] as RequestInit).body as string)).toEqual({
			task: 'check'
		});
	});
});

describe('the HTTP service (job logs)', () => {
	it('reads a page of the job log after a line number', async () => {
		const page = { items: [], has_more: false, truncated: false, purged: false };
		const fetchFn = vi.fn().mockResolvedValue(jsonResponse(page));
		const service = await loadService(fetchFn);

		const result = await service.getJobLogs('j 1', { after_seq: 40, limit: 100 });
		expect(result).toEqual(page);
		const url = String(fetchFn.mock.calls[0][0]);
		expect(url).toContain('/api/v1/jobs/j%201/logs');
		expect(url).toContain('after_seq=40');
		expect(url).toContain('limit=100');
	});
});
