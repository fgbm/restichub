/*
 * RFC 9457 problem documents.
 *
 * The control plane answers a failed request with a problem document. The UI
 * shows the `title` and the `code`, and logs the `requestId`. A validation
 * problem holds the `field` that failed and an optional `reason` code, so a form
 * shows the message next to the input.
 */
import { t, tErrorCode, tErrorReason } from '#lib/i18n/index.ts';
import type { components } from './schema';
import type { Blockers } from './types';

export interface ApiProblem {
	status: number;
	code: string;
	title: string;
	requestId?: string;
	type?: string;
	field?: string;
	reason?: string;
	/** The records that block a delete. */
	blockers?: Blockers;
	/** The repository that holds a taken location. */
	existingName?: string;
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
		const wire = body as Partial<components['schemas']['ProblemBody']>;
		const title = asString(wire.title) ?? defaultTitle(status);
		return {
			status: typeof wire.status === 'number' ? wire.status : status,
			code: asString(wire.code) ?? `http_${status}`,
			title,
			requestId: asString(wire.request_id),
			type: asString(wire.type),
			field: asString(wire.field),
			reason: asString(wire.reason),
			blockers: wire.blockers ?? undefined,
			existingName: asString(wire.existing_name)
		};
	}
	return {
		status,
		code: `http_${status}`,
		title: defaultTitle(status)
	};
}

/**
 * The message of a field of a validation problem, or `undefined`.
 *
 * The function returns a message only when the problem names the field. A form
 * passes the name of each input.
 */
export function fieldError(
	problem: ApiProblem | null | undefined,
	field: string
): string | undefined {
	if (!problem || problem.field !== field) return undefined;
	if (problem.code === 'repository_location_taken') {
		return t('error.repository_location_taken', { name: problem.existingName ?? '' });
	}
	const byCode = problem.code === 'invalid_argument' ? undefined : tErrorCode(problem.code);
	return tErrorReason(problem.reason) ?? byCode ?? t('error.field');
}

/**
 * True when the problem names one of the given fields.
 *
 * A form uses the function to decide whether to show the message next to an
 * input or the general alert. A problem of a field outside the form (for example
 * `body`) gives the general alert.
 */
export function namesField(
	problem: ApiProblem | null | undefined,
	fields: readonly string[]
): boolean {
	return Boolean(problem?.field && fields.includes(problem.field));
}

function defaultTitle(status: number): string {
	if (status === 401) return 'Unauthorized';
	if (status === 403) return 'Forbidden';
	if (status === 404) return 'Not found';
	if (status >= 500) return 'Internal server error';
	return 'Request failed';
}
