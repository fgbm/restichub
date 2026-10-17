<script lang="ts">
	import { onMount } from 'svelte';
	import { beforeNavigate, goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import type {
		ApiProblem,
		HookPhase,
		ScriptFailurePolicy,
		ScriptInterpreter,
		ScriptParameterKind,
		ScriptTemplate,
		ScriptTemplateParameter
	} from '#lib/api/index.ts';
	import CodeEditor from './CodeEditor.svelte';
	import ProfileSection from './ProfileSection.svelte';
	import { RecordPage, SummaryPanel } from './layout/index.ts';
	import { Alert, Button, ConfirmDialog, DataTable, Icon, Select, TextField } from './ui/index.ts';
	import type { KeyValueItem, SelectOption, TableColumn } from './ui/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { changedAt, plural } from '#lib/list-format.ts';
	import { INTERPRETERS, PHASES, templateDescription, templateName } from '#lib/scripts.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	let {
		templateId = null,
		copyFrom = null
	}: {
		templateId?: string | null;
		/** A new template starts as a copy of this template. */
		copyFrom?: string | null;
	} = $props();

	interface ParameterDraft {
		key: number;
		name: string;
		label: string;
		kind: ScriptParameterKind;
		defaultValue: string;
	}

	type SectionKey = 'basics' | 'run' | 'parameters' | 'body';

	let loading = $state(true);
	let loadError = $state<string | null>(null);
	let existing = $state<ScriptTemplate | null>(null);
	let saving = $state(false);
	let saved = false;
	let formError = $state<string | null>(null);
	let formProblem = $state<ApiProblem | null>(null);
	let baseline = $state('');
	let open = $state<Record<SectionKey, boolean>>({
		basics: true,
		run: true,
		parameters: true,
		body: true
	});

	let name = $state('');
	let description = $state('');
	let interpreter = $state<ScriptInterpreter>('bash');
	let phase = $state<HookPhase>('before');
	let body = $state('');
	let timeoutSeconds = $state('');
	let failurePolicy = $state<ScriptFailurePolicy>('abort_backup');
	let parameters = $state<ParameterDraft[]>([]);
	let nextKey = 0;

	let deleteOpen = $state(false);
	let deleting = $state(false);

	const canWrite = $derived(getSession()?.role === 'admin');
	// A built-in template is read-only: a copy changes it.
	const readonly = $derived(Boolean(existing?.builtin) || !canWrite);

	const interpreterOptions = $derived<SelectOption[]>(
		INTERPRETERS.map((value) => ({ value, label: t(`scripts.interpreter.${value}`) }))
	);
	const phaseOptions = $derived<SelectOption[]>(
		PHASES.map((value) => ({ value, label: t(`profiles.hooks.phase.${value}` as MessageKey) }))
	);
	const policyOptions = $derived<SelectOption[]>([
		{ value: 'abort_backup', label: t('scripts.policy.abort_backup') },
		{ value: 'continue', label: t('scripts.policy.continue') }
	]);
	const kindOptions = $derived<SelectOption[]>(
		(['string', 'integer', 'boolean', 'secret_ref'] as const).map((value) => ({
			value,
			label: t(`scriptTemplates.parameter.type.${value}`)
		}))
	);

	function upperFirst(text: string): string {
		return text.charAt(0).toLocaleUpperCase() + text.slice(1);
	}

	function kindLabel(kind: ScriptParameterKind): string {
		return t(`scriptTemplates.parameter.type.${kind}`);
	}

	function draftParameter(parameter?: ScriptTemplateParameter): ParameterDraft {
		nextKey += 1;
		return {
			key: nextKey,
			name: parameter?.name ?? '',
			label: parameter?.label ?? '',
			kind: parameter?.type ?? 'string',
			defaultValue: parameter?.default ?? ''
		};
	}

	function fill(template: ScriptTemplate): void {
		name = templateName(template);
		description = templateDescription(template);
		interpreter = template.interpreter;
		phase = template.phase;
		body = template.body;
		timeoutSeconds = template.timeout_seconds > 0 ? String(template.timeout_seconds) : '';
		failurePolicy = template.failure_policy;
		parameters = template.parameters.map((parameter) => draftParameter(parameter));
	}

	function parameterList(): ScriptTemplateParameter[] {
		return parameters
			.filter((parameter) => parameter.name.trim().length > 0)
			.map((parameter) => {
				const item: ScriptTemplateParameter = {
					name: parameter.name.trim(),
					label: parameter.label.trim() || parameter.name.trim(),
					type: parameter.kind
				};
				if (parameter.kind !== 'secret_ref' && parameter.defaultValue !== '') {
					item.default = parameter.defaultValue;
				}
				return item;
			});
	}

	function snapshot(): string {
		return JSON.stringify([
			name,
			description,
			interpreter,
			phase,
			body,
			timeoutSeconds,
			failurePolicy,
			parameterList()
		]);
	}

	const dirty = $derived(!loading && !readonly && baseline !== '' && snapshot() !== baseline);

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	function problemOf(problem: unknown): ApiProblem | null {
		if (problem instanceof ApiError) return problem.problem;
		if (problem && typeof problem === 'object' && 'problem' in problem) {
			return (problem as { problem: ApiProblem }).problem;
		}
		return null;
	}

	async function load(): Promise<void> {
		try {
			const templates = await service.listScriptTemplates();
			if (templateId) {
				const found = templates.find((item) => item.id === templateId);
				if (!found) {
					loadError = t('scriptTemplates.notFound');
					return;
				}
				existing = found;
				fill(found);
			} else {
				const source = copyFrom ? templates.find((item) => item.id === copyFrom) : undefined;
				if (source) {
					fill(source);
					name = t('scriptTemplates.copyName', { name: templateName(source) });
				}
			}
			baseline = snapshot();
		} catch (problem) {
			loadError = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	onMount(load);

	beforeNavigate(({ cancel }) => {
		if (dirty && !saved && !window.confirm(t('rp.unsaved'))) cancel();
	});

	function toList(): void {
		void goto(resolve('/(app)/settings/scripts/templates'));
	}

	function openCopy(): void {
		if (!existing) return;
		const target = `${resolve('/(app)/settings/scripts/templates/new')}?from=${encodeURIComponent(existing.id)}`;
		// The address is the resolved path of the new template page with a query.
		// eslint-disable-next-line svelte/no-navigation-without-resolve
		void goto(target);
	}

	async function save(): Promise<void> {
		if (readonly) return;
		saving = true;
		formError = null;
		formProblem = null;
		try {
			const timeout = Number.parseInt(timeoutSeconds, 10);
			const shared = {
				name: name.trim(),
				description: description.trim(),
				interpreter,
				phase,
				body,
				timeout_seconds: Number.isFinite(timeout) && timeout > 0 ? timeout : 0,
				failure_policy: failurePolicy,
				parameters: parameterList()
			};
			if (existing) await service.updateScriptTemplate(existing.id, shared);
			else await service.createScriptTemplate(shared);
			pushToast('success', t(existing ? 'scriptTemplates.updated' : 'scriptTemplates.created'));
			saved = true;
			toList();
		} catch (problem) {
			formProblem = problemOf(problem);
			formError = formProblem?.field ? null : messageOf(problem);
		} finally {
			saving = false;
		}
	}

	async function confirmDelete(): Promise<void> {
		if (!existing) return;
		deleting = true;
		try {
			await service.deleteScriptTemplate(existing.id);
			pushToast('success', t('scriptTemplates.deleted'));
			deleteOpen = false;
			saved = true;
			toList();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			deleting = false;
		}
	}

	function addParameter(): void {
		parameters = [...parameters, draftParameter()];
	}

	function removeParameter(key: number): void {
		parameters = parameters.filter((parameter) => parameter.key !== key);
	}

	const lineCount = $derived(body === '' ? 0 : body.split('\n').length);

	const columns = $derived<TableColumn<ParameterDraft>[]>([
		{ key: 'name', header: t('scripts.parameter.name'), sortable: false, render: nameCell },
		{
			key: 'label',
			header: t('scriptTemplates.parameter.label'),
			sortable: false,
			render: labelCell
		},
		{ key: 'kind', header: t('scripts.parameter.kind'), sortable: false, render: kindCell },
		{
			key: 'default',
			header: t('scriptTemplates.parameter.default'),
			sortable: false,
			render: defaultCell
		}
	]);

	const summaryItems = $derived<KeyValueItem[]>([
		{
			key: 'source',
			label: t('scriptTemplates.source'),
			value: existing?.builtin ? t('scriptTemplates.builtin') : t('scriptTemplates.own')
		},
		{
			key: 'interpreter',
			label: t('scriptTemplates.interpreter'),
			value: t(`scripts.interpreter.${interpreter}`)
		},
		{
			key: 'phase',
			label: t('scriptTemplates.phase'),
			value: upperFirst(t(`templates.phase.${phase}`))
		},
		{
			key: 'policy',
			label: t('scripts.failurePolicy'),
			value: upperFirst(
				t(failurePolicy === 'abort_backup' ? 'templates.meta.abort' : 'templates.meta.continue')
			)
		},
		...(existing && !existing.builtin && (existing.updated_at ?? existing.created_at)
			? [
					{
						key: 'changed',
						label: t('templates.changed'),
						value: changedAt(existing.updated_at ?? existing.created_at ?? '')
					}
				]
			: [])
	]);

	// The names that the script reads as environment variables; a secret stays out of the body.
	const variableItems = $derived<KeyValueItem[]>(
		parameters
			.filter((parameter) => parameter.name.trim() !== '')
			.map((parameter) => ({
				key: `variable-${parameter.key}`,
				label: parameter.name.trim(),
				value: kindLabel(parameter.kind),
				render: parameter.kind === 'secret_ref' ? secretValue : undefined
			}))
	);

	const sectionSummaries = $derived({
		basics: name.trim() || t('templates.unnamed'),
		run: `${t(`scripts.interpreter.${interpreter}`)} · ${t(`templates.phase.${phase}`)}`,
		parameters:
			parameters.length > 0
				? plural('templates.parameterCount', parameters.length)
				: t('templates.meta.noParameters'),
		body: t('templates.lines', { n: lineCount })
	});
</script>

{#snippet secretValue()}
	<span class="secret"
		><Icon name="lock" size="xs" />{t('scriptTemplates.parameter.type.secret_ref')}</span
	>
{/snippet}

{#snippet nameCell(parameter: ParameterDraft)}
	{@const index = parameters.indexOf(parameter)}
	<TextField
		aria-label={t('scripts.parameter.name')}
		mono
		error={fieldError(formProblem, `parameters[${index}].name`)}
		bind:value={parameter.name}
		{readonly}
	/>
{/snippet}

{#snippet labelCell(parameter: ParameterDraft)}
	{@const index = parameters.indexOf(parameter)}
	<TextField
		aria-label={t('scriptTemplates.parameter.label')}
		error={fieldError(formProblem, `parameters[${index}].label`)}
		bind:value={parameter.label}
		{readonly}
	/>
{/snippet}

{#snippet kindCell(parameter: ParameterDraft)}
	<Select
		aria-label={t('scripts.parameter.kind')}
		options={kindOptions}
		bind:value={parameter.kind}
		disabled={readonly}
	/>
{/snippet}

{#snippet defaultCell(parameter: ParameterDraft)}
	{@const index = parameters.indexOf(parameter)}
	<TextField
		aria-label={t('scriptTemplates.parameter.default')}
		mono
		error={fieldError(formProblem, `parameters[${index}].default`)}
		bind:value={parameter.defaultValue}
		readonly={readonly || parameter.kind === 'secret_ref'}
	/>
{/snippet}

<RecordPage
	title={templateId
		? name || (existing ? templateName(existing) : '')
		: t('scriptTemplates.newTitle')}
	description={t('scriptTemplates.formDescription')}
	{loading}
	error={loadError}
	onsubmit={() => void save()}
	save={readonly ? undefined : { dirty, saving, create: !templateId, oncancel: toList }}
>
	{#snippet actions()}
		{#if existing && canWrite}
			<Button variant={existing.builtin ? 'primary' : 'secondary'} icon="copy" onclick={openCopy}>
				{t('scriptTemplates.copy')}
			</Button>
			{#if !existing.builtin}
				<Button
					variant="ghost"
					icon="trash"
					class="btn--quiet-danger"
					title={t('scriptTemplates.delete')}
					aria-label={t('scriptTemplates.delete')}
					onclick={() => (deleteOpen = true)}
				/>
			{/if}
		{/if}
	{/snippet}
	{#snippet banners()}
		{#if existing?.builtin}
			<Alert variant="inline" icon="lock" title={t('scriptTemplates.builtinNote')} />
		{/if}
		{#if formError}
			<Alert tone="danger" title={formError} />
		{/if}
	{/snippet}
	{#snippet side()}
		<SummaryPanel title={t('templates.summary')} items={summaryItems} />
		<SummaryPanel title={t('scriptTemplates.variables')} items={variableItems} />
	{/snippet}

	<ProfileSection
		id="basics"
		title={t('scriptTemplates.section.basics')}
		summary={sectionSummaries.basics}
		state="on"
		required
		bind:open={open.basics}
	>
		<TextField
			id="template-name"
			label={t('scriptTemplates.name')}
			error={fieldError(formProblem, 'name')}
			bind:value={name}
			{readonly}
			required
		/>
		<TextField
			id="template-description"
			label={t('scriptTemplates.description.label')}
			error={fieldError(formProblem, 'description')}
			bind:value={description}
			{readonly}
		/>
	</ProfileSection>

	<ProfileSection
		id="run"
		title={t('scriptTemplates.section.run')}
		summary={sectionSummaries.run}
		state="on"
		bind:open={open.run}
	>
		<div class="grid">
			<Select
				id="template-interpreter"
				label={t('scriptTemplates.interpreter')}
				options={interpreterOptions}
				bind:value={interpreter}
				disabled={readonly}
			/>
			<Select
				id="template-phase"
				label={t('scriptTemplates.phase')}
				options={phaseOptions}
				bind:value={phase}
				disabled={readonly}
			/>
			<Select
				id="template-policy"
				label={t('scripts.failurePolicy')}
				options={policyOptions}
				bind:value={failurePolicy}
				disabled={readonly}
			/>
			<TextField
				id="template-timeout"
				type="number"
				label={t('scripts.timeout')}
				help={t('scripts.timeoutHelp')}
				error={fieldError(formProblem, 'timeout_seconds')}
				bind:value={timeoutSeconds}
				{readonly}
			/>
		</div>
	</ProfileSection>

	<ProfileSection
		id="parameters"
		title={t('scripts.parameters')}
		summary={sectionSummaries.parameters}
		state="on"
		bind:open={open.parameters}
	>
		<p class="muted">{t('scriptTemplates.parametersHint')}</p>
		{#if parameters.length > 0}
			<div class="params">
				<DataTable
					{columns}
					rows={parameters}
					rowKey={(parameter) => String(parameter.key)}
					paginated={false}
					actions={readonly
						? undefined
						: (parameter) => [
								{
									id: 'remove',
									label: t('scriptTemplates.parameter.remove'),
									icon: 'trash',
									danger: true,
									onSelect: () => removeParameter(parameter.key)
								}
							]}
				/>
			</div>
		{:else}
			<p class="muted">{t('scripts.parametersEmpty')}</p>
		{/if}
		{#if !readonly}
			<div>
				<Button variant="ghost" icon="plus" onclick={addParameter}>
					{t('scriptTemplates.parameter.add')}
				</Button>
			</div>
		{/if}
	</ProfileSection>

	<ProfileSection
		id="body"
		title={t('scripts.body')}
		summary={sectionSummaries.body}
		state="on"
		required
		bind:open={open.body}
	>
		<CodeEditor
			id="template-body"
			label={t('scripts.body')}
			help={t('scripts.bodyHelp')}
			error={fieldError(formProblem, 'body')}
			bind:value={body}
			{readonly}
			required
		/>
	</ProfileSection>
</RecordPage>

<ConfirmDialog
	bind:open={deleteOpen}
	title={t('scriptTemplates.deleteTitle')}
	message={t('scriptTemplates.deleteBody', { name: existing ? templateName(existing) : '' })}
	confirmLabel={t('scriptTemplates.delete')}
	tone="danger"
	loading={deleting}
	onConfirm={confirmDelete}
/>

<style>
	.grid {
		display: grid;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		gap: var(--rh-space-3);
	}

	@media (max-width: 720px) {
		.grid {
			grid-template-columns: minmax(0, 1fr);
		}
	}

	.muted {
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	/* A narrow screen scrolls the parameter table instead of squeezing its fields. */
	.params :global(td .rh-input),
	.params :global(td .rh-select) {
		min-width: 8rem;
	}

	.secret {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-1);
	}
</style>
