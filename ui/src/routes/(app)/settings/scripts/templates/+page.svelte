<script lang="ts">
	import { onMount } from 'svelte';
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
	import { SettingsLayout } from '#lib/components/index.ts';
	import CodeEditor from '#lib/components/CodeEditor.svelte';
	import {
		Alert,
		Button,
		ConfirmDialog,
		Dialog,
		FilterBar,
		ListView,
		Select,
		TextField
	} from '#lib/components/ui/index.ts';
	import type { FilterFacet, RowAction, SelectOption } from '#lib/components/ui/index.ts';
	import { scriptTemplateMeta } from '#lib/template-item.ts';
	import { ENTITY_ICONS } from '#lib/vocabulary.ts';
	import { facetCounts, matchesSearch, splitList } from '#lib/filters.ts';
	import { queryFilters } from '#lib/stores/query-filters.svelte.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { INTERPRETERS, PHASES, templateDescription, templateName } from '#lib/scripts.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	type ParameterDraft = {
		name: string;
		label: string;
		kind: ScriptParameterKind;
		defaultValue: string;
	};

	let templates = $state<ScriptTemplate[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let formOpen = $state(false);
	// A built-in template opens read-only; the user makes a copy to change it.
	let viewing = $state(false);
	let editing = $state<ScriptTemplate | null>(null);
	let saving = $state(false);
	let formError = $state<string | null>(null);
	let formProblem = $state<ApiProblem | null>(null);

	let name = $state('');
	let description = $state('');
	let interpreter = $state<ScriptInterpreter>('bash');
	let phase = $state<HookPhase>('before');
	let body = $state('');
	let timeoutSeconds = $state('');
	let failurePolicy = $state<ScriptFailurePolicy>('abort_backup');
	let parameters = $state<ParameterDraft[]>([]);

	let deleteTarget = $state<ScriptTemplate | null>(null);
	let deleteOpen = $state(false);
	let deleting = $state(false);

	const interpreterOptions = $derived<SelectOption[]>(
		INTERPRETERS.map((value) => ({
			value,
			label: t(`scripts.interpreter.${value}` as MessageKey)
		}))
	);
	const phaseOptions = $derived<SelectOption[]>(
		PHASES.map((value) => ({
			value,
			label: t(`profiles.hooks.phase.${value}` as MessageKey)
		}))
	);
	const policyOptions = $derived<SelectOption[]>([
		{ value: 'abort_backup', label: t('scripts.policy.abort_backup') },
		{ value: 'continue', label: t('scripts.policy.continue') }
	]);
	const kindOptions = $derived<SelectOption[]>(
		(['string', 'integer', 'boolean', 'secret_ref'] as const).map((value) => ({
			value,
			label: t(`scriptTemplates.parameter.type.${value}` as MessageKey)
		}))
	);

	const filters = queryFilters(['q', 'interpreter', 'phase', 'source', 'sort', 'order'] as const);

	const facets = $derived<FilterFacet[]>([
		{
			key: 'interpreter',
			label: t('templates.facet.interpreter'),
			options: facetCounts(templates, (template) => [template.interpreter]).map((option) => ({
				...option,
				label: t(`scripts.interpreter.${option.value}` as MessageKey)
			}))
		},
		{
			key: 'phase',
			label: t('templates.facet.phase'),
			options: facetCounts(templates, (template) => [template.phase]).map((option) => ({
				...option,
				label: t(`templates.phase.${option.value as HookPhase}`)
			}))
		}
	]);

	const matching = $derived(
		templates.filter((template) => {
			const { q, interpreter, phase } = filters.value;
			if (interpreter && !splitList(interpreter).includes(template.interpreter)) return false;
			if (phase && !splitList(phase).includes(template.phase)) return false;
			return matchesSearch(q, [templateName(template), templateDescription(template)]);
		})
	);

	// The own templates come first when there are any.
	const ownCount = $derived(templates.filter((template) => !template.builtin).length);
	const states = $derived({
		key: 'source',
		default: ownCount > 0 ? 'own' : 'builtin',
		options: (['own', 'builtin'] as const).map((value) => ({
			value,
			label: t(value === 'own' ? 'templates.tab.own' : 'templates.tab.builtin'),
			count: matching.filter((template) => template.builtin === (value === 'builtin')).length
		}))
	});

	const sort = $derived({
		default: 'name',
		options: [{ value: 'name', label: t('templates.sort.name') }]
	});

	const shown = $derived.by(() => {
		const builtin = (filters.value.source || states.default) === 'builtin';
		const factor = filters.value.order === 'desc' ? -1 : 1;
		return matching
			.filter((template) => template.builtin === builtin)
			.sort((a, b) => factor * templateName(a).localeCompare(templateName(b)));
	});

	const filtered = $derived(
		templates.length > 0 &&
			Boolean(filters.value.q || filters.value.interpreter || filters.value.phase)
	);

	function resetFilters(): void {
		filters.update({ q: '', source: '', interpreter: '', phase: '' });
	}

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
		loading = true;
		error = null;
		try {
			templates = await service.listScriptTemplates();
		} catch (problem) {
			error = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	onMount(load);

	function resetForm(): void {
		name = '';
		description = '';
		interpreter = 'bash';
		phase = 'before';
		body = '';
		timeoutSeconds = '';
		failurePolicy = 'abort_backup';
		parameters = [];
		formError = null;
		formProblem = null;
	}

	function fill(template: ScriptTemplate): void {
		name = templateName(template);
		description = templateDescription(template);
		interpreter = template.interpreter;
		phase = template.phase;
		body = template.body;
		timeoutSeconds = template.timeout_seconds > 0 ? String(template.timeout_seconds) : '';
		failurePolicy = template.failure_policy;
		parameters = template.parameters.map((parameter) => ({
			name: parameter.name,
			label: parameter.label,
			kind: parameter.type,
			defaultValue: parameter.default ?? ''
		}));
	}

	function openCreate(): void {
		editing = null;
		viewing = false;
		resetForm();
		formOpen = true;
	}

	function openEdit(template: ScriptTemplate): void {
		editing = template;
		viewing = template.builtin;
		resetForm();
		fill(template);
		formOpen = true;
	}

	function openCopy(template: ScriptTemplate): void {
		editing = null;
		viewing = false;
		resetForm();
		fill(template);
		name = t('scriptTemplates.copyName', { name: templateName(template) });
		formOpen = true;
	}

	function addParameter(): void {
		parameters = [...parameters, { name: '', label: '', kind: 'string', defaultValue: '' }];
	}

	function removeParameter(index: number): void {
		parameters = parameters.filter((_, position) => position !== index);
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

	async function save(): Promise<void> {
		if (viewing) return;
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
			if (editing) await service.updateScriptTemplate(editing.id, shared);
			else await service.createScriptTemplate(shared);
			pushToast('success', t(editing ? 'scriptTemplates.updated' : 'scriptTemplates.created'));
			formOpen = false;
			resetForm();
			await load();
		} catch (problem) {
			formProblem = problemOf(problem);
			formError = formProblem?.field ? null : messageOf(problem);
		} finally {
			saving = false;
		}
	}

	async function confirmDelete(): Promise<void> {
		if (!deleteTarget) return;
		deleting = true;
		try {
			await service.deleteScriptTemplate(deleteTarget.id);
			pushToast('success', t('scriptTemplates.deleted'));
			deleteOpen = false;
			deleteTarget = null;
			await load();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			deleting = false;
		}
	}

	function actionsFor(template: ScriptTemplate): RowAction[] {
		return [
			{
				id: 'copy',
				label: t('scriptTemplates.copy'),
				icon: 'copy',
				onSelect: () => openCopy(template)
			},
			{
				id: 'delete',
				label: t('scriptTemplates.delete'),
				icon: 'trash',
				danger: true,
				hidden: template.builtin,
				onSelect: () => {
					deleteTarget = template;
					deleteOpen = true;
				}
			}
		];
	}

	const dialogTitle = $derived(
		viewing
			? t('scriptTemplates.viewTitle')
			: editing
				? t('scriptTemplates.editTitle')
				: t('scriptTemplates.addTitle')
	);
</script>

<SettingsLayout
	title={t('scriptTemplates.title')}
	description={t('scriptTemplates.description')}
	count={loading ? undefined : templates.length}
>
	{#snippet actions()}
		<Button variant="primary" icon="plus" onclick={openCreate}>{t('scriptTemplates.add')}</Button>
	{/snippet}

	<ListView
		label={t('scriptTemplates.title')}
		items={shown}
		itemKey={(template) => template.id}
		icon={() => ({ name: ENTITY_ICONS.scriptTemplate })}
		title={(template) => templateName(template)}
		onOpen={openEdit}
		meta={scriptTemplateMeta}
		actions={actionsFor}
		moreLabel={(template) => t('scriptTemplates.moreActions', { name: templateName(template) })}
		{loading}
		{error}
		filtered={filtered || (templates.length > 0 && shown.length === 0)}
		onReset={resetFilters}
		emptyIcon={ENTITY_ICONS.scriptTemplate}
		emptyTitle={t('scriptTemplates.emptyTitle')}
		emptyDescription={t('scriptTemplates.emptyDescription')}
	>
		{#snippet emptyActions()}
			<Button variant="primary" onclick={openCreate}>{t('scriptTemplates.addFirst')}</Button>
		{/snippet}
		{#snippet toolbar()}
			<FilterBar
				label={t('scriptTemplates.filters')}
				value={filters.value}
				search={{
					key: 'q',
					label: t('hosts.search'),
					placeholder: t('scriptTemplates.searchPlaceholder')
				}}
				{states}
				{facets}
				{sort}
				onchange={filters.update}
				onreset={resetFilters}
			/>
		{/snippet}
	</ListView>
</SettingsLayout>

<Dialog
	bind:open={formOpen}
	size="xl"
	title={dialogTitle}
	description={t('scriptTemplates.formDescription')}
	onsubmit={save}
>
	{#if formError}
		<Alert tone="danger" title={formError} />
	{/if}
	{#if viewing}
		<Alert variant="inline" icon="lock" title={t('scriptTemplates.builtinNote')} />
	{/if}
	<div class="form">
		<TextField
			id="template-name"
			label={t('scriptTemplates.name')}
			error={fieldError(formProblem, 'name')}
			bind:value={name}
			readonly={viewing}
			required
		/>
		<TextField
			id="template-description"
			label={t('scriptTemplates.description.label')}
			error={fieldError(formProblem, 'description')}
			bind:value={description}
			readonly={viewing}
		/>
		<div class="grid">
			<Select
				id="template-interpreter"
				label={t('scriptTemplates.interpreter')}
				options={interpreterOptions}
				bind:value={interpreter}
				disabled={viewing}
			/>
			<Select
				id="template-phase"
				label={t('scriptTemplates.phase')}
				options={phaseOptions}
				bind:value={phase}
				disabled={viewing}
			/>
			<TextField
				id="template-timeout"
				type="number"
				label={t('scripts.timeout')}
				help={t('scripts.timeoutHelp')}
				error={fieldError(formProblem, 'timeout_seconds')}
				bind:value={timeoutSeconds}
				readonly={viewing}
			/>
			<Select
				id="template-policy"
				label={t('scripts.failurePolicy')}
				options={policyOptions}
				bind:value={failurePolicy}
				disabled={viewing}
			/>
		</div>
		<CodeEditor
			id="template-body"
			label={t('scripts.body')}
			help={t('scripts.bodyHelp')}
			error={fieldError(formProblem, 'body')}
			bind:value={body}
			readonly={viewing}
			required
		/>
		<section class="params">
			<h4 class="params__title">{t('scripts.parameters')}</h4>
			{#each parameters as parameter, index (index)}
				<div class="param">
					<TextField
						label={t('scripts.parameter.name')}
						mono
						error={fieldError(formProblem, `parameters[${index}].name`)}
						bind:value={parameter.name}
						readonly={viewing}
					/>
					<TextField
						label={t('scriptTemplates.parameter.label')}
						error={fieldError(formProblem, `parameters[${index}].label`)}
						bind:value={parameter.label}
						readonly={viewing}
					/>
					<Select
						label={t('scripts.parameter.kind')}
						options={kindOptions}
						bind:value={parameter.kind}
						disabled={viewing}
					/>
					<TextField
						label={t('scriptTemplates.parameter.default')}
						mono
						error={fieldError(formProblem, `parameters[${index}].default`)}
						bind:value={parameter.defaultValue}
						readonly={viewing || parameter.kind === 'secret_ref'}
					/>
					{#if !viewing}
						<Button
							variant="ghost"
							icon="trash"
							aria-label={t('scriptTemplates.parameter.remove')}
							onclick={() => removeParameter(index)}
						/>
					{/if}
				</div>
			{:else}
				<p class="params__help">{t('scripts.parametersEmpty')}</p>
			{/each}
			{#if !viewing}
				<div>
					<Button variant="ghost" icon="plus" onclick={addParameter}>
						{t('scriptTemplates.parameter.add')}
					</Button>
				</div>
			{/if}
		</section>
	</div>
	{#snippet footer()}
		<Button onclick={() => (formOpen = false)}>{t('common.cancel')}</Button>
		{#if viewing && editing}
			<Button
				variant="primary"
				onclick={() => {
					const source = editing;
					formOpen = false;
					if (source) openCopy(source);
				}}
			>
				{t('scriptTemplates.copy')}
			</Button>
		{:else}
			<Button type="submit" variant="primary" loading={saving}>{t('common.save')}</Button>
		{/if}
	{/snippet}
</Dialog>

<ConfirmDialog
	bind:open={deleteOpen}
	title={t('scriptTemplates.deleteTitle')}
	message={t('scriptTemplates.deleteBody', {
		name: deleteTarget ? templateName(deleteTarget) : ''
	})}
	confirmLabel={t('scriptTemplates.delete')}
	tone="danger"
	loading={deleting}
	onConfirm={confirmDelete}
/>

<style>
	.form {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-4);
	}

	.grid {
		display: grid;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		gap: var(--rh-space-3);
	}

	.params {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-2);
	}

	.params__title {
		margin: 0;
		font-size: var(--rh-font-size-sm);
		font-weight: var(--rh-font-weight-semibold);
	}

	.params__help {
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.param {
		display: grid;
		grid-template-columns: minmax(0, 1.1fr) minmax(0, 1fr) 130px minmax(0, 1fr) auto;
		align-items: end;
		gap: var(--rh-space-2);
	}
</style>
