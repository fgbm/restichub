/*
 * The profile parameters in the screens: the rule of a name, the count of uses in a profile draft,
 * the value resolution for a display and the refusal that names the plans (FR-PF-09 to FR-PF-12,
 * ADR-0047).
 *
 * The server resolves a value for the job. The screens resolve it only to show the source and the
 * value that the server would use: the plan, then the host, then the default.
 */
import type { ApiProblem, ParameterKind, PlanParameterValue, ProfileParameter } from '#lib/api/index.ts';
import { blockedView } from '#lib/delete-check.ts';
import type { BlockedView } from '#lib/delete-check.ts';
import type { HostParameterRow } from '#lib/api/index.ts';
import { t } from '#lib/i18n/index.ts';
import { plural } from '#lib/list-format.ts';
import type { ScriptDraft } from '#lib/scripts.ts';

/** The rule of a parameter name, the same as the rule of the server (FR-PF-09). */
const NAME = /^[a-z][a-z0-9_]{0,63}$/;

/** The reason why a name does not fit the rule, or null. */
export function parameterNameError(name: string): string | null {
	const text = name.trim();
	if (text === '' || NAME.test(text)) return null;
	return t('pe.param.nameInvalid');
}

export function kindLabel(kind: ParameterKind): string {
	return t(kind === 'secret' ? 'parameters.kind.secret' : 'parameters.kind.text');
}

/** The count of the parameters of a profile, for example "4 parameters". */
export function parameterCount(count: number): string {
	return plural('parameters.count', count);
}

/** A profile declares parameters. The chip shows the count; a profile without any has no chip. */
export function parameterChip(parameters: readonly ProfileParameter[]): string | null {
	return parameters.length > 0 ? parameterCount(parameters.length) : null;
}

/** The fields of a profile draft that hold a `{{param.<name>}}` reference (FR-PF-10). */
export interface UsageSource {
	paths: string[];
	excludes: string[];
	excludeFiles: string[];
	tags: string[];
	scripts: ScriptDraft[];
}

export interface ParameterUsage {
	paths: number;
	excludes: number;
	excludeFiles: number;
	tags: number;
	scripts: number;
}

function held(values: readonly string[], token: string): number {
	return values.filter((value) => value.includes(token)).length;
}

/** How many fields of the draft take the value of this parameter. */
export function usageOf(name: string, source: UsageSource): ParameterUsage {
	const token = `{{param.${name}}}`;
	return {
		paths: held(source.paths, token),
		excludes: held(source.excludes, token),
		excludeFiles: held(source.excludeFiles, token),
		tags: held(source.tags, token),
		scripts: source.scripts.filter(
			(script) =>
				script.env.some((variable) => variable.kind === 'parameter' && variable.value === name)
		).length
	};
}

// `plural` takes the base of a counted key, so the union of the catalog does not hold it.
const USAGE_LABELS: [keyof ParameterUsage, string][] = [
	['paths', 'parameters.used.paths'],
	['excludes', 'parameters.used.excludes'],
	['excludeFiles', 'parameters.used.excludeFiles'],
	['tags', 'parameters.used.tags'],
	['scripts', 'parameters.used.scripts']
];

/** The column "Used in": "1 path · 1 tag", or a dash for a parameter that nothing uses. */
export function usageText(usage: ParameterUsage): string {
	const parts = USAGE_LABELS.filter(([key]) => usage[key] > 0).map(([key, message]) =>
		plural(message, usage[key])
	);
	return parts.length > 0 ? parts.join(' · ') : t('parameters.usedNone');
}

/** Where a value comes from, in the order that the server applies (FR-PF-10). */
export type ValueSource = 'plan' | 'host' | 'default' | 'none';

export interface ResolvedValue {
	source: ValueSource;
	/** The text of a `text` parameter. Empty for a secret and for a hidden value. */
	text: string;
	/** The name of the referenced secret of a `secret` parameter. A name is not a secret. */
	secretName: string | null;
	/** The server hid the text of this value from the reader (FR-PF-12). */
	hidden: boolean;
}

const NONE: ResolvedValue = { source: 'none', text: '', secretName: null, hidden: false };

/** The value that the server would give to a plan, and the field that holds it. */
export function resolveValue(
	definition: ProfileParameter,
	planValues: readonly PlanParameterValue[],
	host: readonly HostParameterRow[]
): ResolvedValue {
	const plan = planValues.find(
		(value) => value.name === definition.name && value.kind === definition.kind
	);
	if (plan) {
		if (plan.kind === 'secret') {
			return { source: 'plan', text: '', secretName: plan.secret_name ?? '', hidden: false };
		}
		return { source: 'plan', text: plan.value ?? '', secretName: null, hidden: plan.hidden };
	}
	const row = host.find((item) => item.name === definition.name);
	const value = row?.value;
	if (value && value.kind === definition.kind) {
		if (value.kind === 'secret') {
			return {
				source: 'host',
				text: '',
				secretName: value.secret_name ?? null,
				hidden: false
			};
		}
		return {
			source: 'host',
			text: value.value ?? '',
			secretName: null,
			hidden: value.value == null
		};
	}
	if (definition.kind === 'text' && definition.default_value) {
		return {
			source: 'default',
			text: definition.default_value,
			secretName: null,
			hidden: definition.hidden
		};
	}
	return NONE;
}

/** A required parameter that a plan would save without a value (FR-PF-11). */
export function missingValue(
	definition: ProfileParameter,
	planValues: readonly PlanParameterValue[],
	host: readonly HostParameterRow[]
): boolean {
	return definition.required && resolveValue(definition, planValues, host).source === 'none';
}

export interface PlansRefusal {
	/** The names of the parameters without a value. */
	names: string[];
	/** The plans that the refusal names, with their links (FR-PF-11). */
	plans: BlockedView;
}

/**
 * The refusal `parameter_missing_in_plans`: the change leaves these plans without a required
 * value. The answer names the plans and the parameters.
 */
export function plansRefusal(problem: ApiProblem | null): PlansRefusal | null {
	if (!problem || problem.code !== 'parameter_missing_in_plans') return null;
	return {
		names: problem.parameters ?? [],
		plans: blockedView(problem.code, problem.blockers)
	};
}
