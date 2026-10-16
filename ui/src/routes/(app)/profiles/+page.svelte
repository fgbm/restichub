<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { Plan, Profile } from '#lib/api/index.ts';
	import { Page } from '#lib/components/layout/index.ts';
	import { Button, ConfirmDialog, FilterBar, ListView } from '#lib/components/ui/index.ts';
	import type { FilterFacet, ListMeta, RowAction } from '#lib/components/ui/index.ts';
	import { facetCounts, matchesSearch, splitList } from '#lib/filters.ts';
	import { queryFilters } from '#lib/stores/query-filters.svelte.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { plural } from '#lib/list-format.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';
	import { ENTITY_ICONS } from '#lib/vocabulary.ts';

	let profiles = $state<Profile[]>([]);
	let repositories = $state<{ id: string; name: string }[]>([]);
	let plans = $state<Plan[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let deleteTarget = $state<Profile | null>(null);
	let deleteOpen = $state(false);
	let deleting = $state(false);

	const canWrite = $derived(['admin', 'operator'].includes(getSession()?.role ?? ''));
	const repositoryNames = $derived(new Map(repositories.map((item) => [item.id, item.name])));
	const planCounts = $derived.by(() => {
		const counts = new Map<string, number>();
		for (const plan of plans) counts.set(plan.profile_id, (counts.get(plan.profile_id) ?? 0) + 1);
		return counts;
	});

	function repositoryName(profile: Profile): string {
		return repositoryNames.get(profile.repository_id) ?? profile.repository_id;
	}

	function planCount(profile: Profile): number {
		return planCounts.get(profile.id) ?? 0;
	}

	const filters = queryFilters(['q', 'repository', 'tag', 'state', 'sort', 'order'] as const);

	const facets = $derived<FilterFacet[]>([
		{
			key: 'repository',
			label: t('profiles.repository'),
			options: facetCounts(profiles, (profile) => [profile.repository_id]).map((option) => ({
				...option,
				label: repositoryNames.get(option.value) ?? option.value
			}))
		},
		{
			key: 'tag',
			label: t('profiles.tags'),
			options: facetCounts(profiles, (profile) => profile.tags)
		}
	]);

	const matching = $derived(
		profiles.filter((profile) => {
			const { q, repository, tag } = filters.value;
			if (!matchesSearch(q, [profile.name, repositoryName(profile), ...profile.tags])) return false;
			if (repository && !splitList(repository).includes(profile.repository_id)) return false;
			if (tag && !splitList(tag).some((item) => profile.tags.includes(item))) return false;
			return true;
		})
	);

	const states = $derived({
		default: 'all',
		options: [
			{ value: 'all', label: t('profiles.tab.all'), count: matching.length },
			{
				value: 'unused',
				label: t('profiles.tab.unused'),
				count: matching.filter((profile) => planCount(profile) === 0).length
			}
		]
	});

	const sort = $derived({
		default: 'name',
		options: [
			{ value: 'name', label: t('profiles.sort.name') },
			{ value: 'changed', label: t('profiles.sort.changed'), order: 'desc' as const }
		]
	});

	const shown = $derived.by(() => {
		const list = matching.filter(
			(profile) => filters.value.state !== 'unused' || planCount(profile) === 0
		);
		const byChange = filters.value.sort === 'changed';
		const order = filters.value.order || (byChange ? 'desc' : 'asc');
		const factor = order === 'desc' ? -1 : 1;
		const key = (profile: Profile) => (byChange ? profile.updated_at : profile.name);
		return [...list].sort((a, b) => factor * key(a).localeCompare(key(b)));
	});

	const filtered = $derived(
		profiles.length > 0 &&
			Boolean(filters.value.q || filters.value.repository || filters.value.tag || filters.value.state)
	);

	function resetFilters(): void {
		filters.update({ q: '', repository: '', tag: '', state: '' });
	}

	function firstPath(profile: Profile): { first: string; total: number } | null {
		const paths = profile.paths;
		const all = [
			...(paths.all ?? []),
			...(paths.windows ?? []),
			...(paths.linux ?? []),
			...(paths.macos ?? [])
		];
		return all.length > 0 ? { first: all[0], total: all.length } : null;
	}

	const RETENTION: [keyof NonNullable<Profile['retention']>, MessageKey][] = [
		['keep_last', 'profiles.short.last'],
		['keep_hourly', 'profiles.short.hourly'],
		['keep_daily', 'profiles.short.daily'],
		['keep_weekly', 'profiles.short.weekly'],
		['keep_monthly', 'profiles.short.monthly'],
		['keep_yearly', 'profiles.short.yearly']
	];

	function retentionShort(profile: Profile): string {
		const retention = profile.retention;
		if (!retention) return '';
		const parts = RETENTION.filter(([key]) => Number(retention[key] ?? 0) > 0).map(
			([key, message]) => t(message, { count: Number(retention[key]) })
		);
		if (retention.keep_within) {
			parts.push(t('profiles.short.within', { value: retention.keep_within }));
		}
		return parts.join(' · ');
	}

	function meta(profile: Profile): ListMeta[] {
		const path = firstPath(profile);
		const count = planCount(profile);
		return [
			repositoryName(profile),
			path
				? {
						text: path.total > 1 ? `${path.first} +${path.total - 1}` : path.first,
						mono: true
					}
				: null,
			retentionShort(profile),
			count > 0 ? plural('profiles.plans', count) : t('profiles.noPlans')
		];
	}

	function actionsFor(profile: Profile): RowAction[] {
		if (!canWrite) return [];
		const used = planCount(profile) > 0;
		return [
			{
				id: 'copy',
				label: t('profiles.copy'),
				icon: 'copy',
				onSelect: () =>
					void goto(`${resolve('/(app)/profiles/new')}?from=${encodeURIComponent(profile.id)}`)
			},
			{
				id: 'delete',
				label: t('profiles.delete'),
				hint: used ? t('profiles.deleteInUse') : undefined,
				icon: 'trash',
				danger: true,
				disabled: used,
				onSelect: () => {
					deleteTarget = profile;
					deleteOpen = true;
				}
			}
		];
	}

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			const [profileList, repositoryList, planList] = await Promise.all([
				service.listProfiles(),
				service.listRepositories(),
				service.listPlans()
			]);
			profiles = profileList;
			repositories = repositoryList;
			plans = planList;
		} catch (problem) {
			error = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	onMount(load);

	function openCreate(): void {
		void goto(resolve('/(app)/profiles/new'));
	}

	async function confirmDelete(): Promise<void> {
		if (!deleteTarget) return;
		deleting = true;
		try {
			await service.deleteProfile(deleteTarget.id);
			pushToast('success', t('profiles.deleted'));
			deleteOpen = false;
			deleteTarget = null;
			await load();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			deleting = false;
		}
	}
</script>

<Page
	title={t('profiles.title')}
	description={t('profiles.description')}
	count={loading ? undefined : profiles.length}
>
	{#snippet actions()}
		{#if canWrite}
			<Button variant="primary" icon="plus" onclick={openCreate}>{t('profiles.add')}</Button>
		{/if}
	{/snippet}

	<ListView
		label={t('profiles.title')}
		items={shown}
		itemKey={(profile) => profile.id}
		icon={() => ({ name: ENTITY_ICONS.profile })}
		pill={(profile) =>
			planCount(profile) === 0 ? { tone: 'neutral', label: t('profiles.unused') } : null}
		title={(profile) => profile.name}
		href={(profile) => resolve('/(app)/profiles/[id]', { id: profile.id })}
		{meta}
		actions={actionsFor}
		moreLabel={(profile) => t('profiles.moreActions', { name: profile.name })}
		{loading}
		{error}
		{filtered}
		onReset={resetFilters}
		emptyIcon={ENTITY_ICONS.profile}
		emptyTitle={t('profiles.emptyTitle')}
		emptyDescription={t('profiles.emptyDescription')}
	>
		{#snippet emptyActions()}
			{#if canWrite}
				<Button variant="primary" onclick={openCreate}>{t('profiles.addFirst')}</Button>
			{/if}
		{/snippet}
		{#snippet toolbar()}
			<FilterBar
				label={t('profiles.filters')}
				value={filters.value}
				search={{
					key: 'q',
					label: t('hosts.search'),
					placeholder: t('profiles.searchPlaceholder')
				}}
				{states}
				{facets}
				{sort}
				onchange={filters.update}
				onreset={resetFilters}
			/>
		{/snippet}
	</ListView>

	<ConfirmDialog
		bind:open={deleteOpen}
		title={t('profiles.deleteTitle')}
		message={t('profiles.deleteBody', { name: deleteTarget?.name ?? '' })}
		confirmLabel={t('profiles.delete')}
		tone="danger"
		loading={deleting}
		onConfirm={confirmDelete}
	/>
</Page>
