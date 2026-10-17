// Mirrors validate_password in crates/restichub-cp/src/password.rs; password-policy.test.ts
// compares the two.
export const MIN_PASSWORD_CHARS = 12;
export const MAX_PASSWORD_BYTES = 1024;

export type PasswordProblem = 'too_short' | 'too_long';

/** The length as the server counts it: Unicode scalar values, not UTF-16 units. */
export function passwordChars(value: string): number {
	return [...value].length;
}

export function passwordProblem(value: string): PasswordProblem | null {
	if (passwordChars(value) < MIN_PASSWORD_CHARS) return 'too_short';
	if (new TextEncoder().encode(value).length > MAX_PASSWORD_BYTES) return 'too_long';
	return null;
}
