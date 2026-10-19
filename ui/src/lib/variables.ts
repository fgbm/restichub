/*
 * The variables of a profile template, for the list that opens after `{{` (FR-PF-03, FR-PF-10).
 *
 * The list is a hint only: the server expands the template. The names and the date directives
 * match `crates/restichub-compiler/src/template.rs`. The API sends no list of them, so one
 * constant here names that file.
 */
import type { ParameterKind } from '#lib/api/index.ts';
import { t } from '#lib/i18n/index.ts';
import { plural } from '#lib/list-format.ts';
import { referencedNames } from '#lib/parameters.ts';

/** One built-in variable: `hostname`, `os`, `agent.name`, `date` and `date:<format>` (FR-PF-03). */
export const HOST_VARIABLES = ['hostname', 'os', 'agent.name', 'date', 'date:%Y%m%d'] as const;

type HostVariable = (typeof HOST_VARIABLES)[number];

/** The one line below each built-in variable, with a value of a real host. */
const HOST_DESCRIPTIONS: Record<HostVariable, (now: Date) => string> = {
	hostname: () => t('pe.vars.hostname'),
	os: () => t('pe.vars.os'),
	'agent.name': () => t('pe.vars.agentName'),
	date: (now) => t('pe.vars.date', { value: dateExample('%Y-%m-%d', now) ?? '' }),
	'date:%Y%m%d': (now) => t('pe.vars.dateCompact', { value: dateExample('%Y%m%d', now) ?? '' })
};

/** The directives of a date format in template.rs: `%Y`, `%m`, `%d`, `%H`, `%M`, `%S` and `%%`. */
const DATE_DIRECTIVES: Record<string, (date: Date) => string> = {
	Y: (date) => String(date.getUTCFullYear()).padStart(4, '0'),
	m: (date) => String(date.getUTCMonth() + 1).padStart(2, '0'),
	d: (date) => String(date.getUTCDate()).padStart(2, '0'),
	H: (date) => String(date.getUTCHours()).padStart(2, '0'),
	M: (date) => String(date.getUTCMinutes()).padStart(2, '0'),
	S: (date) => String(date.getUTCSeconds()).padStart(2, '0'),
	'%': () => '%'
};

/** One declared parameter as the list shows it, from `declaredParameters` of the draft. */
export interface VariableParameter {
	name: string;
	kind: ParameterKind;
	description?: string | null;
}

/** A label of a host that a plan of the profile uses: `key=value` as the agent stores it. */
export interface HostLabel {
	key: string;
	value: string;
}

export interface VariableSource {
	parameters: readonly VariableParameter[];
	labels: readonly HostLabel[];
	/** The day of the examples; the run date of the server. */
	now?: Date;
}

export interface VariableOption {
	/** The text that Enter writes after the `{{` of the field. */
	expression: string;
	/** The mono name in the list. */
	label: string;
	description?: string;
	/** The kind chip of a parameter. */
	kind?: ParameterKind;
	disabled?: boolean;
	/** Why Enter does not write the option. It shows below the name. */
	reason?: string;
	/** A muted text at the right of the option, for example "type a key". */
	note?: string;
	/** Enter writes the expression without `}}` and keeps the list open for the next level. */
	partial?: boolean;
}

export interface VariableGroup {
	id: string;
	label: string;
	options: VariableOption[];
}

/** The option text of `{{agent.label.<key>}}`; the same prefix starts the second level. */
const LABEL_PREFIX = 'agent.label.';

/** A single line of the field holds the expression only: no space, `}` or newline after `{{`. */
const EXPRESSION_TEXT = /^[A-Za-z0-9_.:%-]*$/;

/** A label key that a person types. The server accepts any key that a host sets. */
const LABEL_KEY = /^[A-Za-z0-9_.-]+$/;

export interface CaretExpression {
	/** The index of the first `{` of `{{`. */
	start: number;
	/** The text after `{{` and before the caret. */
	typed: string;
}

/** The expression that the caret is in, or null. The list opens only inside `{{`. */
export function caretExpression(value: string, caret: number): CaretExpression | null {
	if (caret < 2) return null;
	const start = value.lastIndexOf('{{', caret - 1);
	if (start < 0) return null;
	// A space before `{{` marks a text and not a variable, so the field keeps its keys.
	if (start > 0 && /\s/.test(value[start - 1])) return null;
	const typed = value.slice(start + 2, caret);
	if (!EXPRESSION_TEXT.test(typed)) return null;
	return { start, typed };
}

/**
 * Replaces the typed part of the expression with the option text. `execCommand` keeps the change in
 * the undo history of the browser, so one Ctrl+Z returns the line; the deprecated call is the only
 * way to do that, and `setRangeText` is the fallback (FR-PF-03).
 */
export function insertVariable(
	element: HTMLTextAreaElement | HTMLInputElement,
	start: number,
	end: number,
	inserted: string
): void {
	element.focus();
	element.setSelectionRange(start, end);
	let done: boolean;
	try {
		done = document.execCommand('insertText', false, inserted);
	} catch {
		done = false;
	}
	if (!done) element.setRangeText(inserted, start, end, 'end');
}

/** The date in the format of template.rs, or null for a directive that the server refuses. */
export function dateExample(format: string, now: Date): string | null {
	let output = '';
	for (let index = 0; index < format.length; index++) {
		const character = format[index];
		if (character !== '%') {
			output += character;
			continue;
		}
		const directive = format[index + 1];
		const write = DATE_DIRECTIVES[directive];
		if (directive === undefined || write === undefined) return null;
		output += write(now);
		index++;
	}
	return output;
}

/** The keys of the labels of the hosts, with the first value as the example of each key. */
export function hostLabels(agents: readonly { labels: string[] }[]): HostLabel[] {
	const found = new Map<string, string>();
	for (const agent of agents) {
		for (const label of agent.labels) {
			const at = label.indexOf('=');
			// A label without `=` names no variable. The server skips it too.
			if (at <= 0) continue;
			const key = label.slice(0, at);
			if (!found.has(key)) found.set(key, label.slice(at + 1));
		}
	}
	return [...found]
		.map(([key, value]) => ({ key, value }))
		.sort((left, right) => left.key.localeCompare(right.key));
}

function parameterOptions(source: VariableSource): VariableOption[] {
	return source.parameters.map((parameter) => ({
		expression: `param.${parameter.name}`,
		label: parameter.name,
		kind: parameter.kind,
		description: parameter.description ?? undefined,
		// Restic arguments are visible in the process list, so a secret never expands (FR-AG-06).
		disabled: parameter.kind === 'secret',
		reason: parameter.kind === 'secret' ? t('pe.vars.secretReason') : undefined
	}));
}

function hostOptions(source: VariableSource): VariableOption[] {
	const now = source.now ?? new Date();
	const options: VariableOption[] = HOST_VARIABLES.map((expression) => ({
		expression,
		label: expression,
		description: HOST_DESCRIPTIONS[expression](now)
	}));
	// `agent.label.…` is a prefix and not a variable: Enter writes it and the second level follows.
	options.splice(options.findIndex((option) => option.expression === 'agent.name') + 1, 0, {
		expression: LABEL_PREFIX,
		label: 'agent.label.…',
		description: t('pe.vars.label'),
		note: t('pe.vars.keyHint'),
		partial: true
	});
	return options;
}

function labelOptions(tail: string, source: VariableSource): VariableOption[] {
	const needle = tail.toLowerCase();
	const options = source.labels
		.filter((label) => label.key.toLowerCase().startsWith(needle))
		.map((label) => ({
			expression: `${LABEL_PREFIX}${label.key}`,
			label: `${LABEL_PREFIX}${label.key}`,
			description: t('pe.vars.labelKnown', { key: label.key, value: label.value })
		}));
	// The list is a hint and not a limit: a key that no host of the plans sets yet goes in with
	// one Enter, and the server refuses it at the build (FR-PF-03).
	if (tail !== '' && LABEL_KEY.test(tail) && !source.labels.some((label) => label.key === tail)) {
		options.push({
			expression: `${LABEL_PREFIX}${tail}`,
			label: `${LABEL_PREFIX}${tail}`,
			description: t('pe.vars.labelTyped', { key: tail })
		});
	}
	return options;
}

function matches(option: VariableOption, needle: string): boolean {
	return (
		option.label.toLowerCase().startsWith(needle) ||
		option.expression.toLowerCase().startsWith(needle)
	);
}

/**
 * The groups of the list for the typed text. The parameters stay above the host variables, and a
 * group without a match does not show (FR-PF-03, FR-PF-10).
 */
export function variableGroups(typed: string, source: VariableSource): VariableGroup[] {
	const needle = typed.toLowerCase();
	if (needle.startsWith(LABEL_PREFIX)) {
		const options = labelOptions(typed.slice(LABEL_PREFIX.length), source);
		return options.length > 0 ? [{ id: 'labels', label: t('pe.vars.group.labels'), options }] : [];
	}
	const parameters = parameterOptions(source).filter((option) => matches(option, needle));
	const hosts = hostOptions(source).filter((option) => matches(option, needle));
	// A free date format is a valid directive sequence after `date:` (FR-PF-03). The fixed example
	// takes its row from `HOST_VARIABLES`.
	if (needle.startsWith('date:')) {
		const format = typed.slice('date:'.length);
		const example = format === '' ? null : dateExample(format, source.now ?? new Date());
		if (example !== null && typed !== 'date:%Y%m%d') {
			hosts.push({
				expression: typed,
				label: typed,
				description: t('pe.vars.dateFree', { value: example })
			});
		}
	}
	return [
		{ id: 'parameters', label: t('pe.vars.group.parameters'), options: parameters },
		{ id: 'host', label: t('pe.vars.group.host'), options: hosts }
	].filter((group) => group.options.length > 0);
}

/** The count of the list when no text filters it, for example "9 variables". */
export function variableCount(count: number): string {
	return plural('pe.vars.count', count);
}

/**
 * The name of a secret parameter that a field references, or null. The server refuses the save,
 * and the field says it at once (AC-PF-09, FR-AG-06).
 */
export function secretReference(value: string, secretNames: readonly string[]): string | null {
	return referencedNames(value).find((name) => secretNames.includes(name)) ?? null;
}
