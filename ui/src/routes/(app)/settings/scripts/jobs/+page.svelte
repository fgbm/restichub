<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import type { ApiProblem, JobTemplate, ScriptTemplate, Secret } from '#lib/api/index.ts';
	import { SettingsLayout } from '#lib/components/index.ts';
	import ScriptsEditor from '#lib/components/ScriptsEditor.svelte';
	import {
		Alert,
		Badge,
		Button,
		ConfirmDialog,
		Dialog,
		DropdownMenu,
		EmptyState,
		Table,
		TextField
	} from '#lib/components/ui/index.ts';
	import type { MenuItem, TableColumn } from '#lib/components/ui/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { draftOf, scriptOf } from '#lib/scripts.ts';
	import type { ScriptDraft } from '#lib/scripts.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	let templates = $state<JobTemplate[]>([]);
	let scriptTemplates = $state<ScriptTemplate[]>([]);
	let secrets = $state<Secret[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let formOpen = $state(false);
	// A built-in template opens read-only; the user makes a copy to change it.
	let viewing = $state(false);
	let editing = $state<JobTemplate | null>(null);
	let saving = $state(false);
	let formError = $state<string | null>(null);
	let formProblem = $state<ApiProblem | null>(null);

	let name = $state('');
	let description = $state('');
	let scripts = $state<ScriptDraft[]>([]);

	let deleteTarget = $state<JobTemplate | null>(null);
	let deleteOpen = $state(false);
	let deleting = $state(false);

	const columns = $derived<TableColumn<JobTemplate>[]>([
		{ key: 'name', header: t('jobTemplates.name'), value: (row) => row.name, render: nameCell },
		{
			key: 'scripts',
			header: t('jobTemplates.scripts'),
			value: (row) => String(row.scripts.length),
			cell: (row) => String(row.scripts.length)
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
			templates = await service.listJobTemplates();
			const [scriptList, secretList] = await Promise.allSettled([
				service.listScriptTemplates(),
				service.listSecrets()
			]);
			scriptTemplates = scriptList.status === 'fulfilled' ? scriptList.value : [];
			secrets = secretList.status === 'fulfilled' ? secretList.value : [];
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
		scripts = [];
		formError = null;
		formProblem = null;
	}

	function fill(template: JobTemplate): void {
		name = template.name;
		description = template.description;
		scripts = template.scripts.map((script) => draftOf(script));
	}

	function openCreate(): void {
		editing = null;
		viewing = false;
		resetForm();
		formOpen = true;
	}

	function openEdit(template: JobTemplate): void {
		editing = template;
		viewing = template.builtin;
		resetForm();
		fill(template);
		formOpen = true;
	}

	function openCopy(template: JobTemplate): void {
		editing = null;
		viewing = false;
		resetForm();
		fill(template);
		name = t('scriptTemplates.copyName', { name: template.name });
		formOpen = true;
	}

	async function save(): Promise<void> {
		if (viewing) return;
		saving = true;
		formError = null;
		formProblem = null;
		try {
			const shared = {
				name: name.trim(),
				description: description.trim(),
				scripts: scripts.map(scriptOf)
			};
			if (editing) await service.updateJobTemplate(editing.id, shared);
			else await service.createJobTemplate(shared);
			pushToast('success', t(editing ? 'jobTemplates.updated' : 'jobTemplates.created'));
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
			await service.deleteJobTemplate(deleteTarget.id);
			pushToast('success', t('jobTemplates.deleted'));
			deleteOpen = false;
			deleteTarget = null;
			await load();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			deleting = false;
		}
	}

	function actionsFor(template: JobTemplate): MenuItem[] {
		const items: MenuItem[] = [
			{
				label: t(template.builtin ? 'scriptTemplates.view' : 'scriptTemplates.edit'),
				icon: template.builtin ? 'eye' : 'settings',
				onSelect: () => openEdit(template)
			},
			{ label: t('jobTemplates.copy'), icon: 'copy', onSelect: () => openCopy(template) }
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
			? t('jobTemplates.viewTitle')
			: editing
				? t('jobTemplates.editTitle')
				: t('jobTemplates.addTitle')
	);
</script>

{#snippet nameCell(template: JobTemplate)}
	<span class="template">
		<span class="template__name">{template.name}</span>
		<span class="template__meta">{template.description}</span>
	</span>
{/snippet}

{#snippet sourceCell(template: JobTemplate)}
	<Badge tone={template.builtin ? 'accent' : 'neutral'}>
		{template.builtin ? t('scriptTemplates.builtin') : t('scriptTemplates.own')}
	</Badge>
{/snippet}

<svelte:head>
	<title>{t('jobTemplates.title')} · {t('app.name')}</title>
</svelte:head>

<SettingsLayout
	title={t('jobTemplates.title')}
	description={t('jobTemplates.description')}
	count={loading ? undefined : templates.length}
>
	{#snippet actions()}
		<Button variant="primary" icon="plus" onclick={openCreate}>{t('jobTemplates.add')}</Button>
	{/snippet}

	{#if empty}
		<EmptyState
			icon="checklist"
			title={t('jobTemplates.emptyTitle')}
			description={t('jobTemplates.emptyDescription')}
			bordered
		>
			<Button variant="primary" icon="plus" onclick={openCreate}>
				{t('jobTemplates.addFirst')}
			</Button>
		</EmptyState>
	{:else}
		<Table
			{columns}
			rows={templates}
			{loading}
			{error}
			rowKey={(template) => template.id}
			filterPlaceholder={t('jobTemplates.searchPlaceholder')}
		>
			{#snippet rowActions(template)}
				<DropdownMenu
					icon="more"
					align="end"
					label={t('scriptTemplates.moreActions', { name: template.name })}
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
	description={t('jobTemplates.formDescription')}
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
			id="job-template-name"
			label={t('jobTemplates.name')}
			error={fieldError(formProblem, 'name')}
			bind:value={name}
			readonly={viewing}
			required
		/>
		<TextField
			id="job-template-description"
			label={t('scriptTemplates.description.label')}
			error={fieldError(formProblem, 'description')}
			bind:value={description}
			readonly={viewing}
		/>
		<ScriptsEditor
			bind:scripts
			bind:secrets
			templates={scriptTemplates}
			jobTemplates={[]}
			readonly={viewing}
			problem={formProblem}
			framed={false}
		/>
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
				{t('jobTemplates.copy')}
			</Button>
		{:else}
			<Button type="submit" variant="primary" loading={saving}>{t('common.save')}</Button>
		{/if}
	{/snippet}
</Dialog>

<ConfirmDialog
	bind:open={deleteOpen}
	title={t('jobTemplates.deleteTitle')}
	message={t('jobTemplates.deleteBody', { name: deleteTarget?.name ?? '' })}
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
</style>
