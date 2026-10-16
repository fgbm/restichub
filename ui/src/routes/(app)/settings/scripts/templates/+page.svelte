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
		Badge,
		Button,
		ConfirmDialog,
		Dialog,
		EmptyState,
		Select,
		Table,
		TextField,
		RowActions
	} from '#lib/components/ui/index.ts';
	import type { MenuItem, SelectOption, TableColumn } from '#lib/components/ui/index.ts';
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

	const columns = $derived<TableColumn<ScriptTemplate>[]>([
		{
			key: 'name',
			header: t('scriptTemplates.name'),
			value: (row) => templateName(row),
			render: nameCell
		},
		{
			key: 'interpreter',
			header: t('scriptTemplates.interpreter'),
			cell: (row) => t(`scripts.interpreter.${row.interpreter}` as MessageKey),
			badge: () => 'neutral'
		},
		{
			key: 'phase',
			header: t('scriptTemplates.phase'),
			cell: (row) => t(`profiles.hooks.phase.${row.phase}` as MessageKey)
		},
		{
			key: 'parameters',
			header: t('scriptTemplates.parameters'),
			value: (row) => String(row.parameters.length),
			cell: (row) => String(row.parameters.length)
		},
		{
			key: 'source',
			header: t('scriptTemplates.source'),
			value: (row) => (row.builtin ? t('scriptTemplates.builtin') : t('scriptTemplates.own')),
			render: sourceCell
		}
	]);

	const empty = $derived(!loading && !error && templates.length === 0);

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

	function actionsFor(template: ScriptTemplate): MenuItem[] {
		const items: MenuItem[] = [
			{
				label: t(template.builtin ? 'scriptTemplates.view' : 'scriptTemplates.edit'),
				icon: template.builtin ? 'eye' : 'settings',
				onSelect: () => openEdit(template)
			},
			{ label: t('scriptTemplates.copy'), icon: 'copy', onSelect: () => openCopy(template) }
		];
		if (!template.builtin) {
			items.push({
				label: t('scriptTemplates.delete'),
				icon: 'octagon',
				tone: 'danger',
				onSelect: () => {
					deleteTarget = template;
					deleteOpen = true;
				}
			});
		}
		return items;
	}

	const dialogTitle = $derived(
		viewing
			? t('scriptTemplates.viewTitle')
			: editing
				? t('scriptTemplates.editTitle')
				: t('scriptTemplates.addTitle')
	);
</script>

{#snippet nameCell(template: ScriptTemplate)}
	<span class="template">
		<span class="template__name">{templateName(template)}</span>
		<span class="template__meta">{templateDescription(template)}</span>
	</span>
{/snippet}

{#snippet sourceCell(template: ScriptTemplate)}
	<Badge tone={template.builtin ? 'accent' : 'neutral'}>
		{template.builtin ? t('scriptTemplates.builtin') : t('scriptTemplates.own')}
	</Badge>
{/snippet}

<svelte:head>
	<title>{t('scriptTemplates.title')} · {t('app.name')}</title>
</svelte:head>

<SettingsLayout
	title={t('scriptTemplates.title')}
	description={t('scriptTemplates.description')}
	count={loading ? undefined : templates.length}
>
	{#snippet actions()}
		<Button variant="primary" icon="plus" onclick={openCreate}>{t('scriptTemplates.add')}</Button>
	{/snippet}

	{#if empty}
		<EmptyState
			icon="terminal"
			title={t('scriptTemplates.emptyTitle')}
			description={t('scriptTemplates.emptyDescription')}
			bordered
		>
			<Button variant="primary" icon="plus" onclick={openCreate}>
				{t('scriptTemplates.addFirst')}
			</Button>
		</EmptyState>
	{:else}
		<Table
			{columns}
			rows={templates}
			{loading}
			{error}
			rowKey={(template) => template.id}
			filterPlaceholder={t('scriptTemplates.searchPlaceholder')}
		>
			{#snippet rowActions(template)}
				<RowActions
					moreLabel={t('scriptTemplates.moreActions', { name: templateName(template) })}
					items={actionsFor(template)}
				/>
			{/snippet}
		</Table>
	{/if}
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
	.template {
		display: flex;
		flex-direction: column;
	}

	.template__name {
		font-weight: var(--rh-font-weight-semibold);
	}

	.template__meta {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

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
