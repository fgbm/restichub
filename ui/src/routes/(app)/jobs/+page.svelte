<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { Job, JobStatus } from '#lib/api/index.ts';
	import {
		Button,
		ConfirmDialog,
		DropdownMenu,
		EmptyState,
		PageHeader,
		Select,
		StatusPill,
		Table
	} from '#lib/components/ui/index.ts';
	import type {
		MenuItem,
		SelectOption,
		StatusTone,
		TableColumn
	} from '#lib/components/ui/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { formatDuration, formatLocalTime } from '#lib/format.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	const STATUSES: JobStatus[] = [
		'queued',
		'sent',
		'running',
		'succeeded',
		'failed',
		'expired',
		'dead_letter'
	];
	const PAGE_SIZE = 50;

	let jobs = $state<Job[]>([]);
	let nextCursor = $state<string | null>(null);
	let loading = $state(true);
	let loadingMore = $state(false);
	let error = $state<string | null>(null);
	let statusFilter = $state('');

	let cancelTarget = $state<Job | null>(null);
	let cancelOpen = $state(false);
	let cancelling = $state(false);

	const canWrite = $derived(['admin', 'operator'].includes(getSession()?.role ?? ''));

	const statusOptions = $derived<SelectOption[]>([
		{ value: '', label: t('jobs.allStatuses') },
		...STATUSES.map((value) => ({ value, label: t(`jobs.status.${value}` as MessageKey) }))
	]);

	const columns = $derived<TableColumn<Job>[]>([
		{ key: 'status', header: t('jobs.status'), value: statusLabel, render: statusCell },
		{ key: 'plan', header: t('jobs.plan'), value: planLabel, render: planCell },
		{
			key: 'host',
			header: t('jobs.host'),
			value: (row) => row.agent_hostname ?? '',
			render: hostCell
		},
		{ key: 'repository', header: t('jobs.repository'), value: (row) => row.repository_name ?? '' },
		{
			key: 'started',
			header: t('jobs.started'),
			value: (row) => row.started_at ?? '',
			render: startedCell
		},
		{
			key: 'duration',
			header: t('jobs.duration'),
			value: (row) => durationOf(row),
			render: durationCell
		}
	]);

	const empty = $derived(!loading && !error && jobs.length === 0 && statusFilter === '');

	// A cancelled job ends as `failed` with the result `cancelled`; the screen shows it apart.
	function isCancelled(job: Job): boolean {
		return job.status === 'failed' && job.result_code === 'cancelled';
	}

	function statusLabel(job: Job): string {
		return t(`jobs.status.${isCancelled(job) ? 'cancelled' : job.status}` as MessageKey);
	}

	function toneOf(job: Job): StatusTone {
		if (isCancelled(job)) return 'cancelled';
		switch (job.status) {
			case 'queued':
				return 'queued';
			case 'sent':
				return 'info';
			case 'running':
				return 'running';
			case 'succeeded':
				return 'success';
			case 'expired':
				return 'warning';
			default:
				return 'failed';
		}
	}

	function planLabel(job: Job): string {
		return job.plan_name ?? t(`jobs.kind.${job.kind}` as MessageKey);
	}

	function reasonOf(job: Job): string | null {
		if (!job.result_message || isCancelled(job)) return null;
		const key = `jobs.reason.${job.result_message}` as MessageKey;
		const text = t(key);
		return text === key ? job.result_message : text;
	}

	function durationOf(job: Job): string {
		if (!job.started_at) return '';
		return formatDuration(job.started_at, job.finished_at ?? new Date().toISOString());
	}

	function cancellable(job: Job): boolean {
		return job.status === 'queued' || job.status === 'sent' || job.status === 'running';
	}

	function actionsFor(job: Job): MenuItem[] {
		if (!canWrite || !cancellable(job)) return [];
		return [
			{
				label: t('jobs.cancel'),
				icon: 'stop',
				tone: 'danger',
				onSelect: () => {
					cancelTarget = job;
					cancelOpen = true;
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
			const page = await service.listJobs({
				limit: PAGE_SIZE,
				status: statusFilter ? (statusFilter as JobStatus) : undefined
			});
			jobs = page.items;
			nextCursor = page.next_cursor ?? null;
		} catch (problem) {
			error = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	async function loadMore(): Promise<void> {
		if (!nextCursor) return;
		loadingMore = true;
		try {
			const page = await service.listJobs({
				limit: PAGE_SIZE,
				status: statusFilter ? (statusFilter as JobStatus) : undefined,
				cursor: nextCursor
			});
			jobs = [...jobs, ...page.items];
			nextCursor = page.next_cursor ?? null;
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			loadingMore = false;
		}
	}

	onMount(load);

	async function confirmCancel(): Promise<void> {
		if (!cancelTarget) return;
		cancelling = true;
		try {
			const job = await service.cancelJob(cancelTarget.id);
			pushToast(
				'success',
				t(job.status === 'queued' || isCancelled(job) ? 'jobs.cancelled' : 'jobs.cancelRequested')
			);
			cancelOpen = false;
			cancelTarget = null;
			await load();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			cancelling = false;
		}
	}
</script>

{#snippet statusCell(job: Job)}
	<span class="job">
		<StatusPill tone={toneOf(job)}>{statusLabel(job)}</StatusPill>
		{#if reasonOf(job)}
			<span class="job__meta job__reason">{reasonOf(job)}</span>
		{/if}
	</span>
{/snippet}

{#snippet planCell(job: Job)}
	<span class="job">
		<span class="job__name">{planLabel(job)}</span>
		{#if job.plan_name}
			<span class="job__meta">{t(`jobs.kind.${job.kind}` as MessageKey)}</span>
		{/if}
	</span>
{/snippet}

{#snippet hostCell(job: Job)}
	<span class="job__host">{job.agent_hostname ?? t('common.none')}</span>
{/snippet}

{#snippet startedCell(job: Job)}
	<span class="job__meta">
		{job.started_at ? formatLocalTime(job.started_at) : '—'}
	</span>
{/snippet}

{#snippet durationCell(job: Job)}
	<span class="job__meta">{durationOf(job) || '—'}</span>
{/snippet}

<svelte:head>
	<title>{t('jobs.title')} · {t('app.name')}</title>
</svelte:head>

<PageHeader
	title={t('jobs.title')}
	description={t('jobs.description')}
	count={loading ? undefined : jobs.length}
>
	<Button icon="refresh" onclick={load}>{t('common.refresh')}</Button>
</PageHeader>

{#if empty}
	<EmptyState
		icon="job"
		title={t('jobs.emptyTitle')}
		description={t('jobs.emptyDescription')}
		bordered
	/>
{:else}
	<Table
		{columns}
		rows={jobs}
		{loading}
		{error}
		rowKey={(job) => job.id}
		paginated={false}
		filterPlaceholder={t('jobs.searchPlaceholder')}
	>
		{#snippet toolbar()}
			<div class="filter">
				<Select
					id="jobs-status"
					aria-label={t('jobs.filterStatus')}
					options={statusOptions}
					bind:value={statusFilter}
					onchange={load}
				/>
			</div>
		{/snippet}
		{#snippet rowActions(job)}
			{#if actionsFor(job).length > 0}
				<DropdownMenu
					icon="more"
					align="end"
					label={t('jobs.moreActions', { name: planLabel(job) })}
					items={actionsFor(job)}
				/>
			{/if}
		{/snippet}
	</Table>
	{#if nextCursor}
		<div class="more">
			<Button loading={loadingMore} onclick={loadMore}>{t('jobs.loadMore')}</Button>
		</div>
	{/if}
{/if}

<ConfirmDialog
	bind:open={cancelOpen}
	title={t('jobs.cancelTitle')}
	message={t('jobs.cancelBody', { name: cancelTarget ? planLabel(cancelTarget) : '' })}
	confirmLabel={t('jobs.cancel')}
	cancelLabel={t('jobs.cancelKeep')}
	tone="danger"
	loading={cancelling}
	onConfirm={confirmCancel}
/>

<style>
	.job {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: var(--rh-space-1);
	}

	.job__name {
		font-weight: var(--rh-font-weight-semibold);
		white-space: nowrap;
	}

	.job__host {
		white-space: nowrap;
	}

	.job__meta {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.job__reason {
		max-width: 22ch;
	}

	.filter {
		min-width: 12rem;
	}

	.more {
		display: flex;
		justify-content: center;
		margin-top: var(--rh-space-4);
	}
</style>
