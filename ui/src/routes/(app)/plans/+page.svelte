<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { Agent, Plan, Profile, Repository } from '#lib/api/index.ts';
	import { Page } from '#lib/components/layout/index.ts';
	import { Button, FilterBar, ListView } from '#lib/components/ui/index.ts';
	import type { FilterFacet, ListMeta, RowAction } from '#lib/components/ui/index.ts';
	import { facetCounts, matchesSearch, splitList } from '#lib/filters.ts';
	import { queryFilters } from '#lib/stores/query-filters.svelte.ts';
	import { getLocale, t, tErrorCode } from '#lib/i18n/index.ts';
	import { withHosts } from '#lib/host-picker.ts';
	import { ago } from '#lib/list-format.ts';
	import { profileDescription } from '#lib/profile-picker.ts';
	import { describeCron } from '#lib/schedule.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';
	import { iconOf, pillOf, planState } from '#lib/vocabulary.ts';

	let plans = $state<Plan[]>([]);
	let profiles = $state<Profile[]>([]);
	let repositories = $state<Repository[]>([]);
	let agents = $state<Agent[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	const canWrite = $derived(['admin', 'operator'].includes(getSession()?.role ?? ''));

	const filters = queryFilters(['q', 'host', 'profile', 'state', 'sort', 'order'] as const);

	const facets = $derived<FilterFacet[]>([
		{
			key: 'host',
			label: t('plans.host'),
			search: true,
			options: withHosts(
				facetCounts(plans, (plan) => [plan.agent_hostname]),
				agents,
				(agent) => agent.hostname
			)
		},
		{
			key: 'profile',
			label: t('plans.profile'),
			search: true,
			options: facetCounts(plans, (plan) => [plan.profile_name]).map((option) => ({
				...option,
				description: describeProfile(option.value)
			}))
		}
	]);

	function describeProfile(name: string) {
		const id = plans.find((plan) => plan.profile_name === name)?.profile_id;
		const profile = profiles.find((item) => item.id === id);
		return profile ? profileDescription(profile, repositories, agents) : undefined;
	}

	const matching = $derived(
		plans.filter((plan) => {
			const { q, host, profile } = filters.value;
			if (!matchesSearch(q, [plan.name, plan.agent_hostname, plan.profile_name])) return false;
			if (host && !splitList(host).includes(plan.agent_hostname)) return false;
			if (profile && !splitList(profile).includes(plan.profile_name)) return false;
			return true;
		})
	);

	const needsAttention = (plan: Plan) =>
		plan.enabled && (plan.rpo_state === 'at_risk' || plan.rpo_state === 'violated');

	const states = $derived({
		default: 'all',
		options: [
			{ value: 'all', label: t('plans.tab.all'), count: matching.length },
			{
				value: 'attention',
				label: t('plans.tab.attention'),
				count: matching.filter(needsAttention).length
			},
			{
				value: 'disabled',
				label: t('plans.tab.disabled'),
				count: matching.filter((plan) => !plan.enabled).length
			}
		]
	});

	const sort = $derived({
		default: 'name',
		options: [
			{ value: 'name', label: t('plans.sort.name') },
			{ value: 'success', label: t('plans.sort.lastSuccess') },
			{ value: 'next', label: t('plans.sort.nextRun') }
		]
	});

	const shown = $derived.by(() => {
		const list = matching.filter((plan) => {
			if (filters.value.state === 'disabled') return !plan.enabled;
			if (filters.value.state === 'attention') return needsAttention(plan);
			return true;
		});
		const factor = filters.value.order === 'desc' ? -1 : 1;
		// A plan that never succeeded is the oldest.
		const key = (plan: Plan) => {
			if (filters.value.sort === 'next') return (plan.enabled && plan.next_run_at) || '9999';
			if (filters.value.sort === 'success') return plan.last_success_at ?? '';
			return plan.name;
		};
		return [...list].sort((a, b) => factor * key(a).localeCompare(key(b)));
	});

	const filtered = $derived(
		plans.length > 0 &&
			Boolean(filters.value.q || filters.value.host || filters.value.profile || filters.value.state)
	);

	// A field in the middle of line 2 starts with a lower-case letter.
	function lowerFirst(text: string): string {
		return text.charAt(0).toLocaleLowerCase(getLocale()) + text.slice(1);
	}

	function resetFilters(): void {
		filters.update({ q: '', host: '', profile: '', state: '' });
	}

	function meta(plan: Plan): ListMeta[] {
		return [
			plan.agent_hostname,
			plan.profile_name,
			{ text: lowerFirst(describeCron(plan.cron, t)), title: `${plan.cron} · ${plan.time_zone}` },
			plan.last_success_at
				? {
						text: t('plans.meta.lastBackup', { time: ago(plan.last_success_at) }),
						time: plan.last_success_at
					}
				: t('plans.meta.noBackup')
		];
	}

	function actionsFor(plan: Plan): RowAction[] {
		if (!canWrite) return [];
		return [
			{ id: 'run', label: t('plans.run'), icon: 'job', onSelect: () => void runNow(plan) },
			{
				id: 'toggle',
				label: t(plan.enabled ? 'plans.disable' : 'plans.enable'),
				icon: 'power',
				onSelect: () => void toggle(plan)
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
			// The profile facet shows the repository of each profile only as a hint.
			[plans, profiles, repositories, agents] = await Promise.all([
				service.listPlans(),
				service.listProfiles().catch(() => []),
				service.listRepositories().catch(() => []),
				service.listAgents().catch(() => [])
			]);
		} catch (problem) {
			error = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	onMount(load);

	function openCreate(): void {
		void goto(resolve('/(app)/plans/new'));
	}

	async function runNow(plan: Plan): Promise<void> {
		try {
			await service.runPlan(plan.id);
			pushToast('success', t('plans.runQueued'));
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		}
	}

	async function toggle(plan: Plan): Promise<void> {
		try {
			await service.updatePlan(plan.id, {
				name: plan.name,
				agent_id: plan.agent_id,
				profile_id: plan.profile_id,
				cron: plan.cron,
				time_zone: plan.time_zone,
				missed_run_policy: plan.missed_run_policy,
				rpo_seconds: plan.rpo_seconds ?? null,
				enabled: !plan.enabled
			});
			pushToast('success', t(plan.enabled ? 'plans.disabledToast' : 'plans.enabledToast'));
			await load();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		}
	}
</script>

<Page
	title={t('plans.title')}
	description={t('plans.description')}
	count={loading ? undefined : plans.length}
>
	{#snippet actions()}
		{#if canWrite}
			<Button variant="primary" icon="plus" onclick={openCreate}>{t('plans.add')}</Button>
		{/if}
	{/snippet}

	<ListView
		label={t('plans.title')}
		items={shown}
		itemKey={(plan) => plan.id}
		icon={(plan) => iconOf(planState(plan))}
		pill={(plan) => pillOf(planState(plan))}
		title={(plan) => plan.name}
		href={(plan) => resolve('/(app)/plans/[id]', { id: plan.id })}
		{meta}
		actions={actionsFor}
		moreLabel={(plan) => t('plans.moreActions', { name: plan.name })}
		{loading}
		{error}
		{filtered}
		onReset={resetFilters}
		emptyIcon="calendar"
		emptyTitle={t('plans.emptyTitle')}
		emptyDescription={t('plans.emptyDescription')}
	>
		{#snippet emptyActions()}
			{#if canWrite}
				<Button variant="primary" onclick={openCreate}>{t('plans.addFirst')}</Button>
			{/if}
		{/snippet}
		{#snippet toolbar()}
			<FilterBar
				label={t('plans.filters')}
				value={filters.value}
				search={{ key: 'q', label: t('hosts.search'), placeholder: t('plans.searchPlaceholder') }}
				{states}
				{facets}
				{sort}
				onchange={filters.update}
				onreset={resetFilters}
			/>
		{/snippet}
	</ListView>
</Page>
