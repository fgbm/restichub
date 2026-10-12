/*
 * The fetch client of the UI.
 *
 * Rules:
 * - same origin only, with `credentials: 'same-origin'`;
 * - the header `X-ResticHub-CSRF: 1` on POST, PUT, PATCH and DELETE;
 * - a problem document becomes a typed `ApiError`;
 * - status 401 redirects to the sign-in page and keeps the current path in `next`;
 * - status 403 throws `ForbiddenError` for a clear message.
 */
import { ApiError, ForbiddenError, NetworkError, UnauthorizedError } from './errors';
import { parseProblem } from './problem';
import type { ApiProblem } from './problem';

export type HttpMethod = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';

export type QueryValue = string | number | boolean | null | undefined;

export interface RequestOptions {
	query?: Record<string, QueryValue>;
	body?: unknown;
	signal?: AbortSignal;
	headers?: Record<string, string>;
}

export interface ApiClientOptions {
	/** API prefix. The control plane serves the API under `/api/v1`. */
	baseUrl?: string;
	fetchFn?: typeof fetch;
	/** Called on status 401 before the error is thrown. */
	onUnauthorized?: (next: string) => void;
}

const CSRF_HEADER = 'X-ResticHub-CSRF';
const UNSAFE_METHODS = new Set<HttpMethod>(['POST', 'PUT', 'PATCH', 'DELETE']);

function currentPath(): string {
	if (typeof window === 'undefined') return '/';
	return window.location.pathname + window.location.search;
}

function defaultRedirect(next: string): void {
	if (typeof window === 'undefined') return;
	window.location.assign(`/sign-in?next=${encodeURIComponent(next)}`);
}

function buildUrl(baseUrl: string, path: string, query?: Record<string, QueryValue>): string {
	const url = `${baseUrl}${path}`;
	if (!query) return url;
	const params = new URLSearchParams();
	for (const [key, value] of Object.entries(query)) {
		if (value === undefined || value === null || value === '') continue;
		params.set(key, String(value));
	}
	const search = params.toString();
	return search ? `${url}?${search}` : url;
}

async function readBody(response: Response): Promise<unknown> {
	const contentType = response.headers.get('content-type') ?? '';
	if (!contentType.includes('json')) return undefined;
	try {
		return await response.json();
	} catch {
		return undefined;
	}
}

export class ApiClient {
	readonly baseUrl: string;
	private readonly fetchFn: typeof fetch;
	private readonly onUnauthorized: (next: string) => void;

	constructor(options: ApiClientOptions = {}) {
		this.baseUrl = options.baseUrl ?? '/api/v1';
		this.fetchFn = options.fetchFn ?? globalThis.fetch.bind(globalThis);
		this.onUnauthorized = options.onUnauthorized ?? defaultRedirect;
	}

	async request<T>(method: HttpMethod, path: string, options: RequestOptions = {}): Promise<T> {
		const headers: Record<string, string> = {
			Accept: 'application/json',
			...options.headers
		};
		if (options.body !== undefined) {
			headers['Content-Type'] = 'application/json';
		}
		if (UNSAFE_METHODS.has(method)) {
			headers[CSRF_HEADER] = '1';
		}

		let response: Response;
		try {
			response = await this.fetchFn(buildUrl(this.baseUrl, path, options.query), {
				method,
				headers,
				credentials: 'same-origin',
				body: options.body === undefined ? undefined : JSON.stringify(options.body),
				signal: options.signal
			});
		} catch (cause) {
			if (cause instanceof DOMException && cause.name === 'AbortError') throw cause;
			throw new NetworkError(cause);
		}

		if (response.ok) {
			if (response.status === 204) return undefined as T;
			const body = await readBody(response);
			return body as T;
		}

		const problem = parseProblem(await readBody(response), response.status);
		if (response.status === 401) {
			this.onUnauthorized(currentPath());
			throw new UnauthorizedError(problem);
		}
		if (response.status === 403) {
			throw new ForbiddenError(problem);
		}
		throw new ApiError(problem);
	}

	get<T>(path: string, options?: RequestOptions): Promise<T> {
		return this.request<T>('GET', path, options);
	}

	post<T>(path: string, body?: unknown, options?: RequestOptions): Promise<T> {
		return this.request<T>('POST', path, { ...options, body });
	}

	put<T>(path: string, body?: unknown, options?: RequestOptions): Promise<T> {
		return this.request<T>('PUT', path, { ...options, body });
	}

	patch<T>(path: string, body?: unknown, options?: RequestOptions): Promise<T> {
		return this.request<T>('PATCH', path, { ...options, body });
	}

	delete<T>(path: string, options?: RequestOptions): Promise<T> {
		return this.request<T>('DELETE', path, options);
	}
}

/** The shared client for the application. */
export const api = new ApiClient();

export type { ApiProblem };
