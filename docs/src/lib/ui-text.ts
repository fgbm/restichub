import { readFileSync } from 'node:fs';
import { join } from 'node:path';

// The catalogs are read as text: a module import would pull in the SvelteKit config of the UI.
const read = (lang: string) => readFileSync(join(import.meta.env.UI_I18N_DIR, `${lang}.ts`), 'utf8');

export type Lang = 'en' | 'ru';

function parse(source: string): Map<string, string> {
	const messages = new Map<string, string>();
	for (const [, key, value] of source.matchAll(/^\t'([^']+)':\s*'((?:[^'\\]|\\.)*)',?$/gm)) {
		messages.set(key, value.replace(/\\(.)/g, '$1'));
	}
	const entries = source.match(/^\t'/gm)?.length ?? 0;
	if (messages.size !== entries) {
		throw new Error(`read ${messages.size} of ${entries} UI messages: the catalog format changed`);
	}
	return messages;
}

const catalogs: Record<Lang, Map<string, string>> = { en: parse(read('en')), ru: parse(read('ru')) };

/** The label that the web interface shows, so the pages never quote a stale text. */
export function uiText(lang: Lang, key: string): string {
	const text = catalogs[lang].get(key);
	if (text === undefined) throw new Error(`the UI has no message ${key}`);
	return text;
}

/** Replaces each `{{message.key}}` with the UI label. */
export function withUiText(lang: Lang, source: string): string {
	return source.replace(/\{\{([\w.]+)\}\}/g, (_, key: string) => uiText(lang, key));
}

export function pageLang(locals: App.Locals): Lang {
	return locals.starlightRoute.lang === 'ru' ? 'ru' : 'en';
}
