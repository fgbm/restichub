<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { Plan } from '#lib/api/index.ts';
	import { Page } from '#lib/components/layout/index.ts';
	import {
		Button,
		ConfirmDialog,
		DataTable,
		EmptyState,
		FilterBar,
		StatusPill
	} from '#lib/components/ui/index.ts';
	import type { FilterFacet, RowAction, TableColumn } from '#lib/components/ui/index.ts';
	import { facetCounts, matchesSearch, splitList } from '#lib/filters.ts';
	import { queryFilters } from '#lib/stores/query-filters.svelte.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { formatLocalTime, formatTimeInZone } from '#lib/format.ts';
	import { describeCron, describeMissedRunPolicy } from '#lib/schedule.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	let plans = $state<Plan[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let deleteTarget = $state<Plan | null>(null);
	let deleteOpen = $state(false);
	let deleting = $state(false);

	const canWrite = $derived(['admin', 'operator'].includes(getSession()?.role ?? ''));

	const columns = $derived<TableColumn<Plan>[]>([
		{ key: 'name', header: t('plans.plan'), truncate: 'sm' },
		{
			key: 'enabled',
			header: t('plans.state'),
			nowrap: true,
			value: (row) => t(row.enabled ? 'plans.enabled' : 'plans.disabled'),
			render: stateCell
		},
		{ key: 'host', header: t('plans.host'), truncate: 'sm', value: (row) => row.agent_hostname },
		{
			key: 'profile',
			header: t('plans.profile'),
			truncate: 'sm',
			value: (row) => row.profile_name
		},
		{
			key: 'schedule',
			header: t('plans.schedule'),
			value: (row) => describeCron(row.cron, t),
			render: scheduleCell
		},
		{
			key: 'next_run_at',
			header: t('plans.nextRun'),
			nowrap: true,
			value: (row) => row.next_run_at ?? '',
			render: nextRunCell
		},
		{
			key: 'missed_run_policy',
			header: t('plans.missedRunPolicy'),
			hideNarrow: true,
			truncate: 'sm',
			value: (row) => describeMissedRunPolicy(row.missed_run_policy, t)
		}
	]);

	const filters = queryFilters(['q', 'host', 'profile', 'state'] as const);

	const facets = $derived<FilterFacet[]>([
		{
			key: 'host',
			label: t('plans.host'),
			options: facetCounts(plans, (plan) => [plan.agent_hostname])
		},
		{
			key: 'profile',
			label: t('plans.profile'),
			options: facetCounts(plans, (plan) => [plan.profile_name])
		},
		{
			key: 'state',
			label: t('plans.state'),
			multiple: false,
			options: (['enabled', 'disabled'] as const).map((value) => ({
				value,
				label: t(value === 'enabled' ? 'plans.enabled' : 'plans.disabled'),
				count: plans.filter((plan) => plan.enabled === (value === 'enabled')).length
			}))
		}
	]);

	const shown = $derived(
		plans.filter((plan) => {
			const { q, host, profile, state } = filters.value;
			if (!matchesSearch(q, [plan.name, plan.agent_hostname, plan.profile_name])) return false;
			if (host && !splitList(host).includes(plan.agent_hostname)) return false;
			if (profile && !splitList(profile).includes(plan.profile_name)) return false;
			if (state && plan.enabled !== (state === 'enabled')) return false;
			return true;
		})
	);

	const empty = $derived(!loading && !error && plans.length === 0);

	function actionsFor(plan: Plan): RowAction[] {
		if (!canWrite) return [];
		return [
			{ id: 'run', label: t('plans.run'), icon: 'activity', onSelect: () => void runNow(plan) },
			{
				id: 'delete',
				label: t('plans.delete'),
				icon: 'octagon',
				danger: true,
				onSelect: () => {
					deleteTarget = plan;
					deleteOpen = true;
				}
			}
		];
	}

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	function nextRunTitle(plan: Plan, value: string): string {
		return `${formatTimeInZone(value, plan.time_zone)} ${plan.time_zone} · ${formatTimeInZone(value, 'UTC')} UTC · ${formatLocalTime(value)} ${t('common.localTime')}`;
	}

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			plans = await service.listPlans();
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

	async function confirmDelete(): Promise<void> {
		if (!deleteTarget) return;
		deleting = true;
		try {
			await service.deletePlan(deleteTarget.id);
			pushToast('success', t('plans.deleted'));
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

{#snippet stateCell(plan: Plan)}
	<StatusPill tone={plan.enabled ? 'success' : 'neutral'}>
		{t(plan.enabled ? 'plans.enabled' : 'plans.disabled')}
	</StatusPill>
{/snippet}

{#snippet scheduleCell(plan: Plan)}
	<span class="plan">
		<span>{describeCron(plan.cron, t)}</span>
		<span class="plan__meta plan__cron">{plan.cron}</span>
	</span>
{/snippet}

{#snippet nextRunCell(plan: Plan)}
	{#if plan.next_run_at}
		<time datetime={plan.next_run_at} title={nextRunTitle(plan, plan.next_run_at)}>
			{formatTimeInZone(plan.next_run_at, plan.time_zone)}
		</time>
	{:else}
		{t('common.none')}
	{/if}
{/snippet}

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

	{#if empty}
		<EmptyState
			icon="calendar"
			title={t('plans.emptyTitle')}
			description={t('plans.emptyDescription')}
			bordered
		>
			{#if canWrite}
				<Button variant="primary" onclick={openCreate}>{t('plans.addFirst')}</Button>
			{/if}
		</EmptyState>
	{:else}
		<DataTable
			{columns}
			rows={shown}
			{loading}
			{error}
			rowKey={(plan) => plan.id}
			rowHref={(plan) => resolve('/(app)/plans/[id]', { id: plan.id })}
			actions={actionsFor}
			moreLabel={(plan) => t('plans.moreActions', { name: plan.name })}
			emptyTitle={t('filter.noMatches')}
			emptyDescription={t('filter.noMatchesDescription')}
		>
			{#snippet toolbar()}
				<FilterBar
					label={t('plans.filters')}
					value={filters.value}
					search={{ key: 'q', label: t('hosts.search'), placeholder: t('plans.searchPlaceholder') }}
					{facets}
					onchange={filters.update}
					onreset={filters.reset}
				/>
			{/snippet}
		</DataTable>
	{/if}

	<ConfirmDialog
		bind:open={deleteOpen}
		title={t('plans.deleteTitle')}
		message={t('plans.deleteBody', { name: deleteTarget?.name ?? '' })}
		confirmLabel={t('plans.delete')}
		tone="danger"
		loading={deleting}
		onConfirm={confirmDelete}
	/>
</Page>

<style>
	.plan {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: var(--rh-space-1);
	}

	.plan__meta {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.plan__cron {
		font-family: var(--rh-font-mono);
	}
</style>
