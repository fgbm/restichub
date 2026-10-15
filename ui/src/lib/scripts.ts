/*
 * Pure helpers of the script editor: the form draft of a script, the conversion to the API shape
 * and the rules that the form can check before it sends (FR-HK-09, FR-HK-12, ADR-0037).
 */
import { getLocale, tOr } from '#lib/i18n/index.ts';
import type {
	Agent,
	HookPhase,
	ScriptDef,
	ScriptEnv,
	ScriptEnvKind,
	ScriptFailurePolicy,
	ScriptInterpreter,
	ScriptTemplate
} from '#lib/api/index.ts';

/** The largest body in bytes that the control plane accepts. */
export const MAX_BODY_BYTES = 64 * 1024;

export const PHASES: HookPhase[] = ['before', 'after', 'on_success', 'on_error', 'finally'];

/** The phases of the second list: they run after the backup. */
export const AFTER_PHASES: HookPhase[] = ['after', 'on_success', 'on_error', 'finally'];

export const INTERPRETERS: ScriptInterpreter[] = ['bash', 'sh', 'powershell', 'cmd'];

export interface EnvDraft {
	name: string;
	kind: ScriptEnvKind;
	value: string;
}

/** One script in the form. `key` identifies the card; it is never sent. */
export interface ScriptDraft {
	key: number;
	name: string;
	interpreter: ScriptInterpreter;
	phase: HookPhase;
	body: string;
	timeoutSeconds: string;
	failurePolicy: ScriptFailurePolicy;
	env: EnvDraft[];
	templateId: string | null;
	/** The card shows its body and its settings. */
	open: boolean;
}

let lastKey = 0;

export function newKey(): number {
	lastKey += 1;
	return lastKey;
}

/** The list of a phase: `before` is the first list, all others are the second list. */
export type Group = 'before' | 'after';

export function groupOf(phase: HookPhase): Group {
	return phase === 'before' ? 'before' : 'after';
}

export function blankScript(group: Group, interpreter: ScriptInterpreter = 'bash'): ScriptDraft {
	return {
		key: newKey(),
		name: '',
		interpreter,
		phase: group === 'before' ? 'before' : 'after',
		body: '',
		timeoutSeconds: '',
		failurePolicy: 'abort_backup',
		env: [],
		templateId: null,
		open: true
	};
}

/** The draft of a stored script. */
export function draftOf(script: ScriptDef, open = false): ScriptDraft {
	return {
		key: newKey(),
		name: script.name,
		interpreter: script.interpreter,
		phase: script.phase,
		body: script.body,
		timeoutSeconds: (script.timeout_seconds ?? 0) > 0 ? String(script.timeout_seconds) : '',
		failurePolicy: script.failure_policy,
		env: (script.env ?? []).map((variable) => ({ ...variable })),
		templateId: script.template_id ?? null,
		open
	};
}

type TemplateKind = 'script' | 'job';
interface TemplateText {
	id: string;
	builtin: boolean;
	name: string;
	description: string;
}

function builtinText(
	kind: TemplateKind,
	template: TemplateText,
	field: 'name' | 'description'
): string {
	if (!template.builtin || getLocale() === 'en') return template[field];
	const slug = template.id.replace(/^builtin:/, '');
	return tOr(`builtin.${kind}.${slug}.${field}`, template[field]);
}

/** The name in the current language. A user-created template keeps its own text, and so does a built-in template without a translation. */
export function templateName(template: TemplateText, kind: TemplateKind = 'script'): string {
	return builtinText(kind, template, 'name');
}

export function templateDescription(template: TemplateText, kind: TemplateKind = 'script'): string {
	return builtinText(kind, template, 'description');
}

/** A new script from a template. The parameters become variables with their defaults. */
export function draftFromTemplate(template: ScriptTemplate): ScriptDraft {
	return {
		key: newKey(),
		name: templateName(template),
		interpreter: template.interpreter,
		phase: template.phase,
		body: template.body,
		timeoutSeconds: template.timeout_seconds > 0 ? String(template.timeout_seconds) : '',
		failurePolicy: template.failure_policy,
		env: template.parameters.map((parameter) => ({
			name: parameter.name,
			kind: parameter.type === 'secret_ref' ? 'secret_ref' : 'string',
			value: parameter.type === 'secret_ref' ? '' : (parameter.default ?? '')
		})),
		templateId: template.id,
		open: true
	};
}

/** The API shape of a draft. */
export function scriptOf(draft: ScriptDraft): ScriptDef {
	const timeout = Number.parseInt(draft.timeoutSeconds, 10);
	const script: ScriptDef = {
		name: draft.name.trim(),
		interpreter: draft.interpreter,
		phase: draft.phase,
		body: draft.body,
		timeout_seconds: Number.isFinite(timeout) && timeout > 0 ? timeout : 0,
		failure_policy: draft.failurePolicy,
		env: draft.env
			.filter((variable) => variable.name.trim().length > 0)
			.map((variable): ScriptEnv => ({
				name: variable.name.trim(),
				kind: variable.kind,
				value: variable.value
			}))
	};
	if (draft.templateId) script.template_id = draft.templateId;
	return script;
}

/** The number of UTF-8 bytes of a text, the unit of the body limit. */
export function byteLength(text: string): number {
	return new TextEncoder().encode(text).length;
}

/** Why a host cannot run a script. */
export type HostProblem = 'disabled' | 'interpreter';

export function hostProblem(agent: Agent, interpreter: ScriptInterpreter): HostProblem | null {
	if (agent.server_scripts !== 'enabled') return 'disabled';
	return agent.script_interpreters.includes(interpreter) ? null : 'interpreter';
}

/** The active hosts that cannot run at least one script of the list. */
export function hostsWithProblems(agents: Agent[], scripts: ScriptDraft[]): Agent[] {
	if (scripts.length === 0) return [];
	return agents.filter(
		(agent) =>
			agent.status === 'active' &&
			scripts.some((script) => hostProblem(agent, script.interpreter) !== null)
	);
}

/** The interpreters that suit the operating system of a host, in the order of the form. */
export function interpretersFor(os: string): ScriptInterpreter[] {
	return os.toLowerCase() === 'windows' ? ['powershell', 'cmd'] : ['bash', 'sh'];
}

/**
 * The command that turns the trust mode on at the host. The host administrator runs it as root or
 * Administrator (ADR-0037).
 */
export function enableCommand(os: string): string {
	return os.toLowerCase() === 'windows'
		? 'restichub-agent.exe trust-server-scripts --enable'
		: 'sudo restichub-agent trust-server-scripts --enable';
}

/**
 * Moves the script with `key` over its neighbour in the same list. Returns the same array when the
 * script is at the edge of its list.
 */
export function move(list: ScriptDraft[], key: number, direction: -1 | 1): ScriptDraft[] {
	const index = list.findIndex((item) => item.key === key);
	if (index < 0) return list;
	const group = groupOf(list[index].phase);
	let target = index + direction;
	while (target >= 0 && target < list.length && groupOf(list[target].phase) !== group) {
		target += direction;
	}
	if (target < 0 || target >= list.length) return list;
	const copy = [...list];
	[copy[index], copy[target]] = [copy[target], copy[index]];
	return copy;
}

/** True when the script has a neighbour to move over in its list. */
export function canMove(list: ScriptDraft[], key: number, direction: -1 | 1): boolean {
	return move(list, key, direction) !== list;
}
