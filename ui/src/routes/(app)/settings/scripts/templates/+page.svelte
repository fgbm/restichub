<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { HookPhase, ScriptTemplate } from '#lib/api/index.ts';
	import { SettingsLayout } from '#lib/components/index.ts';
	import { Button, ConfirmDialog, FilterBar, ListView } from '#lib/components/ui/index.ts';
	import type { FilterFacet, RowAction } from '#lib/components/ui/index.ts';
	import { facetCounts, matchesSearch, splitList } from '#lib/filters.ts';
	import { queryFilters } from '#lib/stores/query-filters.svelte.ts';
	import { hasRole } from '#lib/stores/session.svelte.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { templateDescription, templateName } from '#lib/scripts.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';
	import { scriptTemplateMeta } from '#lib/template-item.ts';
	import { ENTITY_ICONS } from '#lib/vocabulary.ts';

	// An Operator reads the templates; only an Admin writes them (ADR-0037, operation
	// `scripts_write`).
	const canWrite = $derived(hasRole('admin', 'platform_admin'));

	let templates = $state<ScriptTemplate[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let deleteTarget = $state<ScriptTemplate | null>(null);
	let deleteOpen = $state(false);
	let deleting = $state(false);

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

	function openCreate(): void {
		void goto(resolve('/(app)/settings/scripts/templates/new'));
	}

	function openCopy(template: ScriptTemplate): void {
		const target = `${resolve('/(app)/settings/scripts/templates/new')}?from=${encodeURIComponent(template.id)}`;
		// The address is the resolved path of the new template page with a query.
		// eslint-disable-next-line svelte/no-navigation-without-resolve
		void goto(target);
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
		// A copy is a create, so it needs the same right as the create button.
		if (!canWrite) return [];
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
</script>

<SettingsLayout
	title={t('scriptTemplates.title')}
	description={t('scriptTemplates.description')}
	count={loading ? undefined : templates.length}
>
	{#snippet actions()}
		{#if canWrite}
			<Button variant="primary" icon="plus" onclick={openCreate}>{t('scriptTemplates.add')}</Button>
		{/if}
	{/snippet}

	<ListView
		label={t('scriptTemplates.title')}
		items={shown}
		itemKey={(template) => template.id}
		icon={() => ({ name: ENTITY_ICONS.scriptTemplate })}
		title={(template) => templateName(template)}
		href={(template) => resolve('/(app)/settings/scripts/templates/[id]', { id: template.id })}
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
			{#if canWrite}
				<Button variant="primary" onclick={openCreate}>{t('scriptTemplates.addFirst')}</Button>
			{/if}
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
