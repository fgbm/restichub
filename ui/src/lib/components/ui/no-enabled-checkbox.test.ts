import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

// The on/off state of a record is a Switch, not a Checkbox (design/style/ui-principles.md, 5).
const SRC = join(process.cwd(), 'src');

function svelteFiles(dir: string): string[] {
	return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
		const path = join(dir, entry.name);
		if (entry.isDirectory()) return svelteFiles(path);
		return entry.name.endsWith('.svelte') ? [path] : [];
	});
}

describe('on/off state', () => {
	it('binds no Checkbox to an enabled field and gives it no enabled label key', () => {
		const offenders = svelteFiles(SRC).filter((path) =>
			[...readFileSync(path, 'utf8').matchAll(/<Checkbox\b[\s\S]*?\/>/g)].some(
				([tag]) => /bind:checked=\{[^}]*\benabled\b[^}]*\}/.test(tag) || /\.enabled'/.test(tag)
			)
		);
		expect(offenders).toEqual([]);
	});
});
