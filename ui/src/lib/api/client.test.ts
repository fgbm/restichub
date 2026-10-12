import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiClient } from './client';
import { ForbiddenError, UnauthorizedError, ApiError } from './errors';

function jsonResponse(body: unknown, status = 200): Response {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json' }
	});
}

afterEach(() => {
	window.history.pushState({}, '', '/');
});

describe('ApiClient', () => {
	it('adds the CSRF header on unsafe methods only', async () => {
		const fetchFn = vi.fn().mockResolvedValue(jsonResponse({ ok: true }));
		const client = new ApiClient({ fetchFn: fetchFn as unknown as typeof fetch });

		await client.get('/thing');
		await client.post('/thing', { name: 'x' });

		const getInit = fetchFn.mock.calls[0][1] as RequestInit;
		const postInit = fetchFn.mock.calls[1][1] as RequestInit;
		expect((getInit.headers as Record<string, string>)['X-ResticHub-CSRF']).toBeUndefined();
		expect((postInit.headers as Record<string, string>)['X-ResticHub-CSRF']).toBe('1');
		expect(postInit.credentials).toBe('same-origin');
	});

	it('parses a problem document into a typed error', async () => {
		const problem = {
			type: 'about:blank',
			title: 'Validation failed',
			status: 422,
			code: 'validation_failed',
			request_id: 'req-123'
		};
		const fetchFn = vi.fn().mockImplementation(() => jsonResponse(problem, 422));
		const client = new ApiClient({ fetchFn: fetchFn as unknown as typeof fetch });

		await expect(client.post('/users', {})).rejects.toBeInstanceOf(ApiError);
		try {
			await client.post('/users', {});
		} catch (error) {
			expect(error).toBeInstanceOf(ApiError);
			const apiError = error as ApiError;
			expect(apiError.status).toBe(422);
			expect(apiError.code).toBe('validation_failed');
			expect(apiError.problem.title).toBe('Validation failed');
			expect(apiError.problem.requestId).toBe('req-123');
		}
	});

	it('redirects to the sign-in page on status 401', async () => {
		const onUnauthorized = vi.fn();
		const fetchFn = vi
			.fn()
			.mockResolvedValue(
				jsonResponse({ title: 'Unauthorized', status: 401, code: 'unauthorized' }, 401)
			);
		const client = new ApiClient({
			fetchFn: fetchFn as unknown as typeof fetch,
			onUnauthorized
		});

		await expect(client.get('/auth/me')).rejects.toBeInstanceOf(UnauthorizedError);
		expect(onUnauthorized).toHaveBeenCalledTimes(1);
		expect(onUnauthorized.mock.calls[0][0]).toBe(window.location.pathname + window.location.search);
	});

	it('does not redirect on status 401 from the authentication endpoints', async () => {
		const onUnauthorized = vi.fn();
		const fetchFn = vi
			.fn()
			.mockResolvedValue(
				jsonResponse({ title: 'Unauthorized', status: 401, code: 'invalid_credentials' }, 401)
			);
		const client = new ApiClient({
			fetchFn: fetchFn as unknown as typeof fetch,
			onUnauthorized
		});

		// The sign-in, the setup and the password change surface the problem to the form.
		await expect(client.post('/auth/sign-in', {})).rejects.toBeInstanceOf(UnauthorizedError);
		await expect(client.post('/setup', {})).rejects.toBeInstanceOf(UnauthorizedError);
		await expect(client.post('/auth/password', {})).rejects.toBeInstanceOf(UnauthorizedError);
		expect(onUnauthorized).not.toHaveBeenCalled();
	});

	it('does not redirect while the sign-in page is open', async () => {
		window.history.pushState({}, '', '/sign-in');
		const onUnauthorized = vi.fn();
		const fetchFn = vi
			.fn()
			.mockResolvedValue(
				jsonResponse({ title: 'Unauthorized', status: 401, code: 'unauthorized' }, 401)
			);
		const client = new ApiClient({
			fetchFn: fetchFn as unknown as typeof fetch,
			onUnauthorized
		});

		await expect(client.get('/auth/me')).rejects.toBeInstanceOf(UnauthorizedError);
		expect(onUnauthorized).not.toHaveBeenCalled();
	});

	it('throws a forbidden error on status 403', async () => {
		const fetchFn = vi
			.fn()
			.mockResolvedValue(jsonResponse({ title: 'Forbidden', status: 403, code: 'forbidden' }, 403));
		const client = new ApiClient({ fetchFn: fetchFn as unknown as typeof fetch });

		await expect(client.get('/users')).rejects.toBeInstanceOf(ForbiddenError);
	});

	it('returns the parsed body on success and builds the query string', async () => {
		const fetchFn = vi.fn().mockResolvedValue(jsonResponse({ items: [] }));
		const client = new ApiClient({ fetchFn: fetchFn as unknown as typeof fetch });

		const result = await client.get('/audit', {
			query: { limit: 20, action: 'user.update', empty: '' }
		});
		expect(result).toEqual({ items: [] });
		const url = fetchFn.mock.calls[0][0] as string;
		expect(url).toContain('limit=20');
		expect(url).toContain('action=user.update');
		expect(url).not.toContain('empty=');
	});
});
