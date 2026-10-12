/*
 * RFC 9457 problem documents.
 *
 * The control plane answers a failed request with a problem document. The UI
 * shows the `title` and the `code`, and logs the `requestId`.
 */
import type { Problem } from './schema';

/** A problem document as the UI uses it. */
export interface ApiProblem {
	status: number;
	code: string;
	title: string;
	requestId?: string;
	detail?: string;
	type?: string;
	instance?: string;
}

function asString(value: unknown): string | undefined {
	return typeof value === 'string' ? value : undefined;
}

/**
 * Read a problem document from a response body.
 *
 * If the body is not a problem document, build a minimal problem from the
 * HTTP status.
 */
export function parseProblem(body: unknown, status: number): ApiProblem {
	if (body && typeof body === 'object') {
		const wire = body as Partial<Problem>;
		const title = asString(wire.title) ?? defaultTitle(status);
		return {
			status: typeof wire.status === 'number' ? wire.status : status,
			code: asString(wire.code) ?? `http_${status}`,
			title,
			requestId: asString(wire.request_id),
			detail: asString(wire.detail),
			type: asString(wire.type),
			instance: asString(wire.instance)
		};
	}
	return {
		status,
		code: `http_${status}`,
		title: defaultTitle(status)
	};
}

function defaultTitle(status: number): string {
	if (status === 401) return 'Unauthorized';
	if (status === 403) return 'Forbidden';
	if (status === 404) return 'Not found';
	if (status >= 500) return 'Internal server error';
	return 'Request failed';
}
