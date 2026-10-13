/*
 * Typed errors of the API layer.
 *
 * A failed request throws `ApiError` with the problem document. The status 401
 * and 403 have their own classes, because the UI treats them in a special way.
 */
import type { ApiProblem } from './problem';

export class ApiError extends Error {
	readonly problem: ApiProblem;

	constructor(problem: ApiProblem) {
		super(problem.title);
		this.name = 'ApiError';
		this.problem = problem;
	}

	get status(): number {
		return this.problem.status;
	}

	get code(): string {
		return this.problem.code;
	}

	get title(): string {
		return this.problem.title;
	}

	get requestId(): string | undefined {
		return this.problem.requestId;
	}
}

/** The client redirects to the sign-in page. */
export class UnauthorizedError extends ApiError {
	constructor(problem: ApiProblem) {
		super(problem);
		this.name = 'UnauthorizedError';
	}
}

/** The UI shows a clear message. */
export class ForbiddenError extends ApiError {
	constructor(problem: ApiProblem) {
		super(problem);
		this.name = 'ForbiddenError';
	}
}

export class NetworkError extends Error {
	constructor(cause?: unknown) {
		super('Network request failed');
		this.name = 'NetworkError';
		this.cause = cause;
	}
}
