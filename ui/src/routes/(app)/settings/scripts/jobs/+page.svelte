<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import type { ApiProblem, JobTemplate, ScriptTemplate, Secret } from '#lib/api/index.ts';
	import { SettingsLayout } from '#lib/components/index.ts';
	import ScriptsEditor from '#lib/components/ScriptsEditor.svelte';
	import {
		Alert,
		Button,
		ConfirmDialog,
		Dialog,
		FilterBar,
		ListView,
		TextField
	} from '#lib/components/ui/index.ts';
	import type { FilterFacet, RowAction } from '#lib/components/ui/index.ts';
	import type { HookPhase, ScriptInterpreter } from '#lib/api/index.ts';
	import { facetCounts, matchesSearch, splitList } from '#lib/filters.ts';
	import { interpreterLabel, jobTemplateMeta } from '#lib/template-item.ts';
	import { ENTITY_ICONS } from '#lib/vocabulary.ts';
	import { queryFilters } from '#lib/stores/query-filters.svelte.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { draftOf, scriptOf, templateDescription, templateName } from '#lib/scripts.ts';
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

	const filters = queryFilters(['q', 'source', 'interpreter', 'phase', 'sort', 'order'] as const);

	const facets = $derived<FilterFacet[]>([
		{
			key: 'interpreter',
			label: t('templates.facet.interpreter'),
			options: facetCounts(templates, (template) => [
				...new Set(template.scripts.map((script) => script.interpreter))
			]).map((option) => ({
				...option,
				label: interpreterLabel(option.value as ScriptInterpreter)
			}))
		},
		{
			key: 'phase',
			label: t('templates.facet.phase'),
			options: facetCounts(templates, (template) => [
				...new Set(template.scripts.map((script) => script.phase))
			]).map((option) => ({
				...option,
				label: t(`templates.phase.${option.value as HookPhase}`)
			}))
		}
	]);

	const matching = $derived(
		templates.filter((template) => {
			const { q, interpreter, phase } = filters.value;
			if (
				interpreter &&
				!template.scripts.some((script) => splitList(interpreter).includes(script.interpreter))
			) {
				return false;
			}
			if (phase && !template.scripts.some((script) => splitList(phase).includes(script.phase))) {
				return false;
			}
			return matchesSearch(q, [
				templateName(template, 'job'),
				templateDescription(template, 'job')
			]);
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
			.sort((a, b) => factor * templateName(a, 'job').localeCompare(templateName(b, 'job')));
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
		name = templateName(template, 'job');
		description = templateDescription(template, 'job');
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
		name = t('scriptTemplates.copyName', { name: templateName(template, 'job') });
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

	function actionsFor(template: JobTemplate): RowAction[] {
		return [
			{
				id: 'copy',
				label: t('jobTemplates.copy'),
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
			? t('jobTemplates.viewTitle')
			: editing
				? t('jobTemplates.editTitle')
				: t('jobTemplates.addTitle')
	);
</script>

<SettingsLayout
	title={t('jobTemplates.title')}
	description={t('jobTemplates.description')}
	count={loading ? undefined : templates.length}
>
	{#snippet actions()}
		<Button variant="primary" icon="plus" onclick={openCreate}>{t('jobTemplates.add')}</Button>
	{/snippet}

	<ListView
		label={t('jobTemplates.title')}
		items={shown}
		itemKey={(template) => template.id}
		icon={() => ({ name: ENTITY_ICONS.jobTemplate })}
		title={(template) => templateName(template, 'job')}
		onOpen={openEdit}
		meta={jobTemplateMeta}
		actions={actionsFor}
		moreLabel={(template) =>
			t('scriptTemplates.moreActions', { name: templateName(template, 'job') })}
		{loading}
		{error}
		filtered={filtered || (templates.length > 0 && shown.length === 0)}
		onReset={resetFilters}
		emptyIcon={ENTITY_ICONS.jobTemplate}
		emptyTitle={t('jobTemplates.emptyTitle')}
		emptyDescription={t('jobTemplates.emptyDescription')}
	>
		{#snippet emptyActions()}
			<Button variant="primary" onclick={openCreate}>{t('jobTemplates.addFirst')}</Button>
		{/snippet}
		{#snippet toolbar()}
			<FilterBar
				label={t('scriptTemplates.filters')}
				value={filters.value}
				search={{
					key: 'q',
					label: t('hosts.search'),
					placeholder: t('jobTemplates.searchPlaceholder')
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
	message={t('jobTemplates.deleteBody', {
		name: deleteTarget ? templateName(deleteTarget, 'job') : ''
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
</style>
