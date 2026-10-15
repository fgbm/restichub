/*
 * AC-UI-03: a template shows no hard-coded user text, and every catalog key that the code uses
 * exists in both catalogs.
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { Linter } from 'eslint';
import svelteParser from 'svelte-eslint-parser';
import { describe, expect, it } from 'vitest';
import rule from '../../../eslint-rules/no-hardcoded-text.js';
import { en } from './en';
import { ru } from './ru';

function messages(source: string): string[] {
	const linter = new Linter();
	const found = linter.verify(
		source,
		[
			{
				files: ['**/*.svelte'],
				languageOptions: { parser: svelteParser },
				plugins: { local: { rules: { 'no-hardcoded-text': rule as never } } },
				rules: { 'local/no-hardcoded-text': 'error' }
			}
		],
		'sample.svelte'
	);
	return found.map((entry) => entry.message);
}

describe('ac_ui_03 the hard-coded text lint', () => {
	it('ac_ui_03_reports_a_text_node', () => {
		expect(messages('<p>Save changes</p>')).toHaveLength(1);
	});

	it('ac_ui_03_reports_a_text_attribute', () => {
		expect(messages('<input placeholder="Search hosts" />')).toHaveLength(1);
		expect(messages('<button aria-label="Close">x</button>').length).toBeGreaterThan(0);
	});

	it('ac_ui_03_reports_a_string_in_a_shown_expression', () => {
		expect(messages("<p>{ready ? 'Ready' : 'Waiting'}</p>")).toHaveLength(2);
	});

	it('ac_ui_03_reports_a_string_in_an_attribute_expression', () => {
		expect(messages("<i aria-label={'Delete'}></i>")).toHaveLength(1);
		expect(messages("<i title={busy ? 'Wait' : 'Go'}></i>")).toHaveLength(2);
		expect(messages("<i aria-label={t('common.delete')}></i>")).toEqual([]);
	});

	it('ac_ui_03_accepts_a_catalog_text_and_technical_values', () => {
		expect(messages("<p>{t('common.save')}</p>")).toEqual([]);
		expect(messages('<input placeholder="ops@example.com" />')).toEqual([]);
		expect(messages('<input placeholder="30d" />')).toEqual([]);
		expect(messages('<code>restic backup</code>')).toEqual([]);
		expect(messages('<Badge tone={ok ? "success" : "neutral"}>{t("a")}</Badge>')).toEqual([]);
		expect(messages('<style>.rule { color: red; }</style>')).toEqual([]);
	});
});

function sources(directory: string): string[] {
	const files: string[] = [];
	for (const name of readdirSync(directory)) {
		const path = join(directory, name);
		if (statSync(path).isDirectory()) files.push(...sources(path));
		else if (/\.(svelte|ts)$/.test(name) && !/\.test\.ts$/.test(name)) files.push(path);
	}
	return files;
}

describe('ac_ui_03 the catalog keys in use', () => {
	it('ac_ui_03_every_literal_key_of_t_exists_in_both_catalogs', () => {
		const keys = new Set<string>();
		for (const file of sources(join(process.cwd(), 'src'))) {
			const text = readFileSync(file, 'utf8');
			for (const match of text.matchAll(/\bt(?:Or)?\(\s*'([a-zA-Z0-9_.-]+)'/g)) {
				keys.add(match[1]);
			}
		}
		expect(keys.size).toBeGreaterThan(100);
		const missing = [...keys].filter((key) => !(key in en) || !(key in ru));
		expect(missing).toEqual([]);
	});
});
