/*
 * Localization runtime.
 *
 * The catalog selection is reactive. A component that calls `t` updates when
 * the language changes. The language comes from localStorage, then from
 * `navigator.language`, then from English.
 */
import { en, type MessageKey } from './en';
import { ru } from './ru';

export type Locale = 'en' | 'ru';

export const locales: readonly Locale[] = ['en', 'ru'];

const catalogs: Record<Locale, Record<MessageKey, string>> = { en, ru };

const STORAGE_KEY = 'restichub.locale';

function isLocale(value: string | null | undefined): value is Locale {
	return value === 'en' || value === 'ru';
}

function detectLocale(): Locale {
	if (typeof localStorage !== 'undefined') {
		const stored = localStorage.getItem(STORAGE_KEY);
		if (isLocale(stored)) return stored;
	}
	if (typeof navigator !== 'undefined') {
		const languages = navigator.languages ?? [navigator.language];
		if (languages.some((language) => language.toLowerCase().startsWith('ru'))) return 'ru';
	}
	return 'en';
}

function interpolate(template: string, params?: Record<string, string | number>): string {
	if (!params) return template;
	return template.replace(/\{(\w+)\}/g, (match, name: string) => {
		const value = params[name];
		return value === undefined ? match : String(value);
	});
}

let locale = $state<Locale>(detectLocale());

export function getLocale(): Locale {
	return locale;
}

export function setLocale(next: Locale): void {
	locale = next;
	if (typeof localStorage !== 'undefined') localStorage.setItem(STORAGE_KEY, next);
	if (typeof document !== 'undefined') document.documentElement.lang = next;
}

/** Call this after mount. */
export function applyLocale(): void {
	if (typeof document !== 'undefined') document.documentElement.lang = locale;
}

export function t(key: MessageKey, params?: Record<string, string | number>): string {
	return interpolate(catalogs[locale][key], params);
}

/**
 * Translate an API problem code. If the catalog has no message for the code,
 * the caller shows the generic title and the code itself.
 */
export function tErrorCode(code: string | undefined): string | undefined {
	if (!code) return undefined;
	const key = `error.${code}` as MessageKey;
	if (key in en) return t(key);
	return undefined;
}

/**
 * Translate the failure code of a notification send. If the catalog has no message for the code,
 * the caller shows the code itself.
 */
export function tChannelError(code: string | undefined): string | undefined {
	if (!code) return undefined;
	const key = `channels.error.${code}` as MessageKey;
	if (key in en) return t(key);
	return undefined;
}

/**
 * Translate a validation reason code. If the catalog has no message for the
 * code, the caller shows a generic message for the field.
 */
export function tErrorReason(reason: string | undefined): string | undefined {
	if (!reason) return undefined;
	const key = `reason.${reason}` as MessageKey;
	if (key in en) return t(key);
	return undefined;
}

/**
 * Translate an audit action. The server stores a snake_case name. If the catalog
 * has no message for the name, the function returns the raw name.
 */
export function tAuditAction(action: string): string {
	const key = `audit.action.${action}` as MessageKey;
	return key in en ? t(key) : action;
}

/**
 * Translate an audit actor type. If the catalog has no message for the type, the
 * function returns the raw type.
 */
export function tAuditActorType(actorType: string): string {
	const key = `audit.actorType.${actorType}` as MessageKey;
	return key in en ? t(key) : actorType;
}
