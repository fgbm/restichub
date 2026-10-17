import { t } from '#lib/i18n/index.ts';

/**
 * Focuses the first field whose check fails and returns `true`, so a submit handler can stop.
 * Each check is the input id and whether the value is invalid.
 */
export function focusFirstInvalid(checks: [id: string, invalid: boolean][]): boolean {
	const failed = checks.find(([, invalid]) => invalid);
	if (!failed) return false;
	document.getElementById(failed[0])?.focus();
	return true;
}

/** The error of a required field, shown only after a submit attempt. */
export function emptyError(submitted: boolean, value: string): string | undefined {
	return submitted && value === '' ? t('reason.empty') : undefined;
}
