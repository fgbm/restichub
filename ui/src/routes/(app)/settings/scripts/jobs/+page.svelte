<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { HookPhase, JobTemplate, ScriptInterpreter } from '#lib/api/index.ts';
	import { Page } from '#lib/components/layout/index.ts';
	import { Button, ConfirmDialog, FilterBar, ListView } from '#lib/components/ui/index.ts';
	import type { FilterFacet, RowAction } from '#lib/components/ui/index.ts';
	import { facetCounts, matchesSearch, splitList } from '#lib/filters.ts';
	import { queryFilters } from '#lib/stores/query-filters.svelte.ts';
	import { hasRole } from '#lib/stores/session.svelte.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { templateDescription, templateName } from '#lib/scripts.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';
	import { interpreterLabel, jobTemplateMeta } from '#lib/template-item.ts';
	import { ENTITY_ICONS } from '#lib/vocabulary.ts';

	// An Operator reads the templates; only an Admin writes them (ADR-0037, operation
	// `scripts_write`).
	const canWrite = $derived(hasRole('admin', 'platform_admin'));

	let templates = $state<JobTemplate[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

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

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			templates = await service.listJobTemplates();
		} catch (problem) {
			error = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	onMount(load);

	function openCreate(): void {
		void goto(resolve('/(app)/settings/scripts/jobs/new'));
	}

	function openCopy(template: JobTemplate): void {
		const target = `${resolve('/(app)/settings/scripts/jobs/new')}?from=${encodeURIComponent(template.id)}`;
		// The address is the resolved path of the new template page with a query.
		// eslint-disable-next-line svelte/no-navigation-without-resolve
		void goto(target);
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
		// A copy is a create, so it needs the same right as the create button.
		if (!canWrite) return [];
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
</script>

<Page
	title={t('jobTemplates.title')}
	description={t('jobTemplates.description')}
	count={loading ? undefined : templates.length}
>
	{#snippet actions()}
		{#if canWrite}
			<Button variant="primary" icon="plus" onclick={openCreate}>{t('jobTemplates.add')}</Button>
		{/if}
	{/snippet}

	<ListView
		label={t('jobTemplates.title')}
		items={shown}
		itemKey={(template) => template.id}
		icon={() => ({ name: ENTITY_ICONS.jobTemplate })}
		title={(template) => templateName(template, 'job')}
		href={(template) => resolve('/(app)/settings/scripts/jobs/[id]', { id: template.id })}
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
			{#if canWrite}
				<Button variant="primary" onclick={openCreate}>{t('jobTemplates.addFirst')}</Button>
			{/if}
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
</Page>

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
