import { describe, expect, it } from 'vitest';
import { fieldError, namesField, parseProblem, type ApiProblem } from './problem';

function problem(field?: string, reason?: string): ApiProblem {
	return { status: 400, code: 'invalid_argument', title: 'Bad Request', field, reason };
}

describe('namesField', () => {
	it('is true when the problem names a field of the form', () => {
		expect(namesField(problem('username'), ['username', 'password'])).toBe(true);
	});

	it('is false for a field outside the form, so the general alert shows', () => {
		// A problem of the field `body` must not hide the general alert.
		expect(namesField(problem('body'), ['username', 'password'])).toBe(false);
	});

	it('is false when the problem names no field', () => {
		expect(namesField(problem(), ['username'])).toBe(false);
		expect(namesField(null, ['username'])).toBe(false);
		expect(namesField(undefined, ['username'])).toBe(false);
	});
});

describe('fieldError', () => {
	it('returns the reason message of the named field', () => {
		expect(fieldError(problem('username', 'too_long'), 'username')).toBe('This value is too long.');
	});

	it('returns the generic message when the reason is unknown', () => {
		expect(fieldError(problem('username'), 'username')).toBe('Correct this field.');
	});

	it('marks only the field that the conflict names', () => {
		const taken: ApiProblem = {
			status: 409,
			code: 'email_taken',
			title: 'Conflict',
			field: 'email'
		};
		expect(fieldError(taken, 'email')).toBe('This email is taken.');
		expect(fieldError(taken, 'username')).toBeUndefined();
	});

	it('fr_rp_11 names the repository that holds a taken location', () => {
		const taken: ApiProblem = {
			status: 409,
			code: 'repository_location_taken',
			title: 'Conflict',
			field: 'backend.path',
			existingName: 'repo-b'
		};
		expect(fieldError(taken, 'backend.path')).toContain('"repo-b"');
	});

	it('returns undefined for another field', () => {
		expect(fieldError(problem('username'), 'password')).toBeUndefined();
		expect(fieldError(null, 'username')).toBeUndefined();
	});
});

describe('parseProblem', () => {
	it('reads the field and the reason', () => {
		const parsed = parseProblem(
			{
				title: 'Bad Request',
				status: 400,
				code: 'invalid_argument',
				field: 'name',
				reason: 'empty'
			},
			400
		);
		expect(parsed.field).toBe('name');
		expect(parsed.reason).toBe('empty');
	});
});
