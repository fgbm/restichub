/*
 * Cached Intl formatters. Creating a formatter costs far more than one call of `format`, and a
 * list formats several values of each item on each update.
 */
import { getLocale } from '#lib/i18n/index.ts';

const cache = new Map<string, unknown>();

function cached<T>(kind: string, options: object, create: (locale: string) => T): T {
	const locale = getLocale();
	const key = `${kind}|${locale}|${JSON.stringify(options)}`;
	let formatter = cache.get(key) as T | undefined;
	if (formatter === undefined) {
		formatter = create(locale);
		cache.set(key, formatter);
	}
	return formatter;
}

export function dateTimeFormat(options: Intl.DateTimeFormatOptions = {}): Intl.DateTimeFormat {
	return cached('date', options, (locale) => new Intl.DateTimeFormat(locale, options));
}

export function numberFormat(options: Intl.NumberFormatOptions = {}): Intl.NumberFormat {
	return cached('number', options, (locale) => new Intl.NumberFormat(locale, options));
}

export function relativeTimeFormat(
	options: Intl.RelativeTimeFormatOptions = {}
): Intl.RelativeTimeFormat {
	return cached('relative', options, (locale) => new Intl.RelativeTimeFormat(locale, options));
}

export function pluralRules(options: Intl.PluralRulesOptions = {}): Intl.PluralRules {
	return cached('plural', options, (locale) => new Intl.PluralRules(locale, options));
}
