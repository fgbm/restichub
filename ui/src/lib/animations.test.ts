import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

// A spinner or a skeleton must keep moving while the main thread renders a list. Only an animation
// of `transform` and `opacity` can run on the compositor; any other property needs the main thread.
const SRC = join(process.cwd(), 'src');
const ALLOWED = new Set(['transform', 'opacity']);

function styleFiles(dir: string): string[] {
	return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
		const path = join(dir, entry.name);
		if (entry.isDirectory()) return styleFiles(path);
		return /\.(svelte|css)$/.test(entry.name) ? [path] : [];
	});
}

/** The properties that each `@keyframes` block of a style sheet animates. */
function keyframeProperties(source: string): Map<string, Set<string>> {
	const found = new Map<string, Set<string>>();
	const start = /@keyframes\s+([\w-]+)\s*\{/g;
	for (let match = start.exec(source); match; match = start.exec(source)) {
		let depth = 1;
		let index = start.lastIndex;
		while (depth > 0 && index < source.length) {
			if (source[index] === '{') depth += 1;
			if (source[index] === '}') depth -= 1;
			index += 1;
		}
		const body = source.slice(start.lastIndex, index - 1);
		const properties = new Set(
			[...body.matchAll(/([a-z-]+)\s*:/g)].map(([, property]) => property)
		);
		found.set(match[1], properties);
	}
	return found;
}

describe('animations', () => {
	it('animate only transform and opacity', () => {
		const offenders: string[] = [];
		let blocks = 0;
		for (const file of styleFiles(SRC)) {
			for (const [name, properties] of keyframeProperties(readFileSync(file, 'utf8'))) {
				blocks += 1;
				for (const property of properties) {
					if (!ALLOWED.has(property)) offenders.push(`${file}: ${name} animates ${property}`);
				}
			}
		}
		expect(blocks).toBeGreaterThan(0);
		expect(offenders).toEqual([]);
	});

	it('finds a property that the compositor cannot run', () => {
		const sheet = '@keyframes shine { to { background-position: -200% 0; } }';
		expect(keyframeProperties(sheet).get('shine')).toEqual(new Set(['background-position']));
	});
});
