import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
	MAX_PASSWORD_BYTES,
	MIN_PASSWORD_CHARS,
	passwordChars,
	passwordProblem
} from './password-policy.ts';

const serverSource = readFileSync(
	join(process.cwd(), '../crates/restichub-cp/src/password.rs'),
	'utf8'
);

function serverConstant(name: string): number {
	const match = serverSource.match(new RegExp(`pub const ${name}: usize = (\\d+);`));
	expect(match, `${name} in password.rs`).not.toBeNull();
	return Number(match?.[1]);
}

describe('password policy', () => {
	it('matches the limits of the control plane', () => {
		expect(MIN_PASSWORD_CHARS).toBe(serverConstant('MIN_PASSWORD_LEN'));
		expect(MAX_PASSWORD_BYTES).toBe(serverConstant('MAX_PASSWORD_BYTES'));
	});

	it('checks the same rules as the control plane and nothing more', () => {
		const body = serverSource.slice(
			serverSource.indexOf('pub fn validate_password'),
			serverSource.indexOf('fn hasher')
		);
		expect(body.match(/return Err\(PasswordError::\w+\)/g)).toEqual([
			'return Err(PasswordError::TooShort)',
			'return Err(PasswordError::TooLong)'
		]);
	});

	it('counts characters, not UTF-16 units', () => {
		expect(passwordChars('😀😀')).toBe(2);
		expect(passwordProblem('😀'.repeat(MIN_PASSWORD_CHARS))).toBeNull();
		expect(passwordProblem('a'.repeat(MIN_PASSWORD_CHARS - 1))).toBe('too_short');
	});

	it('limits the bytes, not the characters', () => {
		expect(passwordProblem('a'.repeat(MAX_PASSWORD_BYTES))).toBeNull();
		expect(passwordProblem('a'.repeat(MAX_PASSWORD_BYTES + 1))).toBe('too_long');
		expect(passwordProblem('я'.repeat(MAX_PASSWORD_BYTES / 2 + 1))).toBe('too_long');
	});
});
