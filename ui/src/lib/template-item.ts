/*
 * Line 2 of the job template and script template items (design/style/ui-principles.md, 2).
 */
import type { HookPhase, JobTemplate, ScriptInterpreter, ScriptTemplate } from '#lib/api/index.ts';
import type { ListMeta } from '#lib/components/ui/index.ts';
import { t } from '#lib/i18n/index.ts';
import { changedAt, plural } from '#lib/list-format.ts';

const PHASES: HookPhase[] = ['before', 'after', 'on_success', 'on_error', 'finally'];

export function interpreterLabel(interpreter: ScriptInterpreter): string {
	return t(`scripts.interpreter.${interpreter}`);
}

function changed(template: JobTemplate | ScriptTemplate): ListMeta {
	if (template.builtin) return t('templates.meta.builtin');
	const at = template.updated_at ?? template.created_at;
	return at ? { text: t('list.changed', { time: changedAt(at) }), time: at } : null;
}

/** "3 scripts: before, after, on error", the interpreters, then the source. */
export function jobTemplateMeta(template: JobTemplate): ListMeta[] {
	const phases = PHASES.filter((phase) =>
		template.scripts.some((script) => script.phase === phase)
	);
	const interpreters = [...new Set(template.scripts.map((script) => script.interpreter))];
	return [
		template.builtin ? t('templates.meta.builtin') : null,
		template.scripts.length > 0
			? t('templates.meta.scripts', {
					count: plural('templates.scriptCount', template.scripts.length),
					phases: phases.map((phase) => t(`templates.phaseShort.${phase}`)).join(', ')
				})
			: t('templates.meta.noScripts'),
		interpreters.map(interpreterLabel).join(', '),
		template.builtin ? null : changed(template)
	];
}

/** The interpreter, the phase, the parameters, the failure policy, then the source. */
export function scriptTemplateMeta(template: ScriptTemplate): ListMeta[] {
	const secrets = template.parameters.filter((parameter) => parameter.type === 'secret_ref').length;
	const values = template.parameters.length - secrets;
	const parameters = [
		values > 0 ? plural('templates.parameterCount', values) : null,
		secrets > 0 ? plural('templates.secretCount', secrets) : null
	].filter(Boolean);
	return [
		interpreterLabel(template.interpreter),
		t(`templates.phase.${template.phase}`),
		parameters.length > 0 ? parameters.join(', ') : t('templates.meta.noParameters'),
		t(
			template.failure_policy === 'abort_backup'
				? 'templates.meta.abort'
				: 'templates.meta.continue'
		),
		changed(template)
	];
}
