/*
 * The rule for the name of a profile, a plan and an alert channel (FR-PF-01). It is the rule of
 * `validate_record_name` in the control plane: free text of 1 to 100 characters, trimmed, without
 * a control or format character. The control plane decides; this check only shows the problem
 * before the request.
 */
import { t } from '#lib/i18n/index.ts';

export const MAX_RECORD_NAME = 100;

// The control characters (Cc), the line and paragraph separators (Zl, Zp) and the format
// characters that `has_forbidden` of restichub-proto lists.
const FORBIDDEN =
	/[\p{Cc}\u{2028}\u{2029}\u{AD}\u{600}-\u{605}\u{61C}\u{6DD}\u{70F}\u{890}-\u{891}\u{8E2}\u{180E}\u{200B}-\u{200F}\u{202A}-\u{202E}\u{2060}-\u{2064}\u{2066}-\u{206F}\u{FEFF}\u{FFF9}-\u{FFFB}\u{110BD}\u{110CD}\u{13430}-\u{1343F}\u{1BCA0}-\u{1BCA3}\u{1D173}-\u{1D17A}\u{E0001}\u{E0020}-\u{E007F}]/u;

export type NameProblem = 'empty' | 'too_long' | 'invalid_character';

export function nameProblem(value: string): NameProblem | null {
	const name = value.trim();
	const length = [...name].length;
	if (length === 0) return 'empty';
	if (length > MAX_RECORD_NAME) return 'too_long';
	if (FORBIDDEN.test(name)) return 'invalid_character';
	return null;
}

/** The message for a name that is filled in and not valid. An empty name has its own message. */
export function nameError(value: string): string | undefined {
	const problem = nameProblem(value);
	return problem && problem !== 'empty' ? t(`reason.${problem}`) : undefined;
}

/**
 * The name of a new plan before the user types one: `<host> | <profile>`, unique among the
 * known plan names without regard to case, and within the length limit.
 */
export function defaultPlanName(host: string, profile: string, taken: readonly string[]): string {
	if (!host.trim() || !profile.trim()) return '';
	const used = new Set(taken.map((name) => name.trim().toLowerCase()));
	const fit = (text: string, room: number) => [...text].slice(0, room).join('').trim();
	const base = `${host.trim()} | ${profile.trim()}`;
	for (let n = 1; ; n += 1) {
		const suffix = n === 1 ? '' : ` (${n})`;
		const candidate = fit(base, MAX_RECORD_NAME - suffix.length) + suffix;
		if (!used.has(candidate.toLowerCase())) return candidate;
	}
}
