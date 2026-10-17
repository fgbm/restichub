// The catalogs use one term for each concept, the one of docs/.../reference/glossary.md.
import { describe, expect, it } from 'vitest';
import { en } from './en';
import { ru } from './ru';

const BANNED_RU: [RegExp, string][] = [
	[/панел[ьиюей] управления/i, 'сервер управления'],
	[/контрольн\S* плоскост/i, 'сервер управления'],
	[/(^|[^а-яё])(узел|узл[а-яё]*)/i, 'хост'],
	[/на связи/i, 'в сети'],
	[/задача «\{what\}»/i, 'задание']
];

const BANNED_EN: [RegExp, string][] = [
	[/control panel/i, 'control plane'],
	[/\bnodes?\b/i, 'host']
];

function offenders(catalog: Record<string, string>, banned: [RegExp, string][]): string[] {
	return Object.entries(catalog).flatMap(([key, text]) =>
		banned.filter(([pattern]) => pattern.test(text)).map(([, term]) => `${key}: use «${term}»`)
	);
}

describe('glossary terms in the catalogs', () => {
	it('the Russian catalog uses no banned synonym', () => {
		expect(offenders(ru, BANNED_RU)).toEqual([]);
	});

	it('the English catalog uses no banned synonym', () => {
		expect(offenders(en, BANNED_EN)).toEqual([]);
	});
});
