import { afterEach, describe, expect, it } from 'vitest';
import { en } from './en';
import { ru } from './ru';
import { setLocale, t, tErrorCode } from './i18n.svelte';

afterEach(() => {
	setLocale('en');
});

describe('i18n catalogs (AC-UI-03)', () => {
	it('ac_ui_03_en_and_ru_have_identical_keys', () => {
		const enKeys = Object.keys(en).sort();
		const ruKeys = Object.keys(ru).sort();
		expect(ruKeys).toEqual(enKeys);
		expect(enKeys.length).toBeGreaterThan(50);
	});

	it('returns the English text by default and the Russian text after a change', () => {
		expect(t('common.save')).toBe('Save');
		setLocale('ru');
		expect(t('common.save')).toBe('Сохранить');
	});

	it('interpolates parameters', () => {
		expect(t('pagination.summary', { from: 1, to: 10, total: 42 })).toBe('Showing 1 to 10 of 42');
	});

	it('maps a known problem code and ignores an unknown one', () => {
		expect(tErrorCode('forbidden')).toBe('You do not have permission for this operation.');
		expect(tErrorCode('unknown_code')).toBeUndefined();
		expect(tErrorCode(undefined)).toBeUndefined();
	});
});
