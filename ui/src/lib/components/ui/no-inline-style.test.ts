import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

// The production policy is `style-src 'self'` without `unsafe-inline` (ADR-0027).
// A `style` attribute needs `unsafe-inline`, so no template holds one.
const SRC = join(process.cwd(), 'src');

function svelteFiles(dir: string): string[] {
	return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
		const path = join(dir, entry.name);
		if (entry.isDirectory()) return svelteFiles(path);
		return entry.name.endsWith('.svelte') ? [path] : [];
	});
}

function markup(source: string): string {
	return source
		.replace(/<script[\s\S]*?<\/script>/g, '')
		.replace(/<style[\s\S]*?<\/style>/g, '')
		.replace(/<!--[\s\S]*?-->/g, '');
}

describe('no inline styles', () => {
	it('finds no style attribute or style directive in a template', () => {
		const offenders = svelteFiles(SRC).filter((path) => {
			const body = markup(readFileSync(path, 'utf8'));
			return /\bstyle\s*=/.test(body) || /\bstyle:/.test(body);
		});
		expect(offenders).toEqual([]);
	});
});
