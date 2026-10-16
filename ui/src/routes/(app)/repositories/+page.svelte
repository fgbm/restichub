<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ApiError, service } from '#lib/api/index.ts';
	import type {
		Job,
		JobResultCode,
		JobStatus,
		MaintenanceTask,
		Repository,
		RepositoryBackendKind
	} from '#lib/api/index.ts';
	import { Page } from '#lib/components/layout/index.ts';
	import {
		Alert,
		Button,
		ConfirmDialog,
		DataTable,
		Dialog,
		EmptyState,
		FilterBar,
		Select,
		StatusPill,
		TextField
	} from '#lib/components/ui/index.ts';
	import type {
		FilterFacet,
		RowAction,
		SelectOption,
		StatusTone,
		TableColumn
	} from '#lib/components/ui/index.ts';
	import { matchesSearch, splitList } from '#lib/filters.ts';
	import { queryFilters } from '#lib/stores/query-filters.svelte.ts';
	import { t, tErrorCode, type MessageKey } from '#lib/i18n/index.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	let repositories = $state<Repository[]>([]);
	let agents = $state<{ id: string; hostname: string; status: string }[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let deleteTarget = $state<Repository | null>(null);
	let deleteOpen = $state(false);
	let deleting = $state(false);

	let unlockTarget = $state<Repository | null>(null);
	let unlockOpen = $state(false);
	let unlockConfirm = $state('');
	let unlocking = $state(false);
	let unlockError = $state<string | null>(null);

	// The init and test-access dialog (FR-RP-02).
	let jobOpen = $state(false);
	let jobRepository = $state<Repository | null>(null);
	let jobKind = $state<'init' | 'test_access'>('init');
	let jobHost = $state('');
	let job = $state<Job | null>(null);
	let jobError = $state<string | null>(null);
	let jobStarting = $state(false);
	let jobTimer: ReturnType<typeof setInterval> | null = null;

	const jobStatusLabels: Record<JobStatus, MessageKey> = {
		queued: 'repositories.job.queued',
		sent: 'repositories.job.sent',
		running: 'repositories.job.running',
		succeeded: 'repositories.job.succeeded',
		failed: 'repositories.job.failed',
		expired: 'repositories.job.expired',
		dead_letter: 'repositories.job.failed'
	};

	const jobResultLabels: Record<JobResultCode, MessageKey> = {
		succeeded: 'repositories.result.succeeded',
		failed: 'repositories.result.failed',
		config_error: 'repositories.result.config_error',
		not_initialized: 'repositories.result.not_initialized',
		access_error: 'repositories.result.access_error',
		cancelled: 'repositories.result.cancelled',
		succeeded_with_warnings: 'repositories.result.succeeded_with_warnings',
		repository_locked: 'repositories.result.repository_locked',
		incompatible: 'repositories.result.incompatible'
	};

	const jobDone = $derived(
		job !== null &&
			(job.status === 'succeeded' || job.status === 'failed' || job.status === 'expired')
	);

	// A job past its expiry with no answer: the poll stops and the dialog says so (MEDIUM 5).
	const jobExpired = $derived(
		job !== null &&
			!jobDone &&
			typeof job.expires_at === 'string' &&
			Date.parse(job.expires_at) <= Date.now()
	);

	const jobAgentOptions = $derived<SelectOption[]>(
		agents
			.filter((agent) => agent.status === 'active')
			.map((agent) => ({ value: agent.id, label: agent.hostname }))
	);

	const canWrite = $derived(getSession()?.role === 'admin');
	const canRun = $derived(getSession()?.role === 'admin' || getSession()?.role === 'operator');
	const agentNames = $derived(new Map(agents.map((agent) => [agent.id, agent.hostname])));

	const backendLabels: Record<RepositoryBackendKind, MessageKey> = {
		s3: 'repositories.backendKind.s3',
		rest: 'repositories.backendKind.rest',
		local: 'repositories.backendKind.local'
	};

	const columns = $derived<TableColumn<Repository>[]>([
		{ key: 'name', header: t('repositories.repository'), truncate: 'sm' },
		{
			key: 'address',
			header: t('rp.section.location'),
			mono: true,
			truncate: 'md',
			hideNarrow: true,
			value: backendAddress
		},
		{
			key: 'backend',
			header: t('repositories.backend'),
			nowrap: true,
			value: (row) => t(backendLabels[row.backend.kind])
		},
		{
			key: 'maintenance_agent_id',
			header: t('repositories.maintenanceHost'),
			value: (row) => maintenanceName(row),
			render: maintenanceCell
		},
		{
			key: 'initialized',
			header: t('repositories.status'),
			value: (row) => statusLabel(row),
			render: statusCell
		}
	]);

	function backendAddress(repository: Repository): string {
		const backend = repository.backend;
		switch (backend.kind) {
			case 's3':
				return `s3://${backend.bucket}/${backend.prefix ?? ''}`.replace(/\/$/, '');
			case 'rest':
				return `rest:${backend.endpoint}`;
			case 'local':
				return backend.path;
		}
	}

	function maintenanceName(repository: Repository): string {
		if (!repository.maintenance_agent_id) return t('repositories.maintenanceHostNone');
		return agentNames.get(repository.maintenance_agent_id) ?? repository.maintenance_agent_id;
	}

	function statusLabel(repository: Repository): string {
		return t(
			repository.initialized
				? 'repositories.status.initialized'
				: 'repositories.status.notInitialized'
		);
	}

	function openCreate(): void {
		void goto(resolve('/(app)/repositories/new'));
	}

	const filters = queryFilters(['q', 'backend', 'state'] as const);

	const facets = $derived<FilterFacet[]>([
		{
			key: 'backend',
			label: t('repositories.backend'),
			options: (Object.keys(backendLabels) as RepositoryBackendKind[]).map((value) => ({
				value,
				label: t(backendLabels[value]),
				count: repositories.filter((repository) => repository.backend.kind === value).length
			}))
		},
		{
			key: 'state',
			label: t('repositories.status'),
			multiple: false,
			options: (['initialized', 'not_initialized'] as const).map((value) => ({
				value,
				label: t(
					value === 'initialized'
						? 'repositories.status.initialized'
						: 'repositories.status.notInitialized'
				),
				count: repositories.filter(
					(repository) => repository.initialized === (value === 'initialized')
				).length
			}))
		}
	]);

	const shown = $derived(
		repositories.filter((repository) => {
			const { q, backend, state } = filters.value;
			if (!matchesSearch(q, [repository.name, backendAddress(repository)])) return false;
			if (backend && !splitList(backend).includes(repository.backend.kind)) return false;
			if (state && repository.initialized !== (state === 'initialized')) return false;
			return true;
		})
	);

	function rowActions(repository: Repository): RowAction[] {
		const maintained = canRun && Boolean(repository.maintenance_agent_id) && repository.initialized;
		return [
			{
				id: 'retention',
				label: t('repositories.runRetention'),
				icon: 'refresh',
				hidden: !maintained,
				onSelect: () => void runTask(repository, 'retention')
			},
			{
				id: 'check',
				label: t('repositories.runCheck'),
				icon: 'refresh',
				hidden: !maintained,
				onSelect: () => void runTask(repository, 'check')
			},
			{
				id: 'init',
				label: t('repositories.initialize'),
				icon: 'plus',
				hidden: !canWrite || repository.initialized,
				onSelect: () => openJob(repository, 'init')
			},
			{
				id: 'test',
				label: t('repositories.testAccess'),
				icon: 'eye',
				hidden: !canWrite,
				onSelect: () => openJob(repository, 'test_access')
			},
			{
				id: 'unlock',
				label: t('repositories.unlockAll'),
				icon: 'lock',
				danger: true,
				hidden: !canWrite || !repository.initialized,
				onSelect: () => openUnlockAll(repository)
			},
			{
				id: 'delete',
				label: t('repositories.delete'),
				icon: 'octagon',
				danger: true,
				hidden: !canWrite,
				onSelect: () => {
					deleteTarget = repository;
					deleteOpen = true;
				}
			}
		];
	}

	const empty = $derived(!loading && !error && repositories.length === 0);

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			const [repositoryList, agentList] = await Promise.all([
				service.listRepositories(),
				service.listAgents()
			]);
			repositories = repositoryList;
			agents = agentList;
		} catch (problem) {
			error = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	onMount(load);

	async function runTask(repository: Repository, task: MaintenanceTask): Promise<void> {
		try {
			await service.runMaintenance(repository.id, task);
			pushToast('success', t('repositories.maintenanceQueued'));
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		}
	}

	function openUnlockAll(repository: Repository): void {
		unlockTarget = repository;
		unlockConfirm = '';
		unlockError = null;
		unlockOpen = true;
	}

	async function submitUnlockAll(): Promise<void> {
		if (!unlockTarget || unlockConfirm !== unlockTarget.name) return;
		unlocking = true;
		unlockError = null;
		try {
			await service.unlockAllRepository(unlockTarget.id, unlockConfirm);
			pushToast('success', t('repositories.unlockAllQueued'));
			unlockOpen = false;
		} catch (problem) {
			unlockError = messageOf(problem);
		} finally {
			unlocking = false;
		}
	}

	async function confirmDelete(): Promise<void> {
		if (!deleteTarget) return;
		deleting = true;
		try {
			await service.deleteRepository(deleteTarget.id);
			pushToast('success', t('repositories.deleted'));
			deleteOpen = false;
			deleteTarget = null;
			await load();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			deleting = false;
		}
	}

	// A closed dialog stops the polling timer.
	$effect(() => {
		if (!jobOpen) stopJobTimer();
	});

	function jobMessage(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		// The mock throws a bare error whose message is a problem code.
		if (problem instanceof Error) {
			const known = tErrorCode(problem.message);
			if (known) return known;
		}
		return t('table.error');
	}

	function openJob(repository: Repository, kind: 'init' | 'test_access'): void {
		jobRepository = repository;
		jobKind = kind;
		// The default host is the maintenance host of the repository (FR-RP-08).
		jobHost =
			repository.maintenance_agent_id ??
			agents.find((agent) => agent.status === 'active')?.id ??
			'';
		job = null;
		jobError = null;
		jobStarting = false;
		jobOpen = true;
	}

	async function startJob(): Promise<void> {
		if (!jobRepository || !jobHost) return;
		jobStarting = true;
		jobError = null;
		try {
			const request = { agent_id: jobHost };
			job =
				jobKind === 'init'
					? await service.initRepository(jobRepository.id, request)
					: await service.testAccessRepository(jobRepository.id, request);
			pollJob();
		} catch (problem) {
			jobError = jobMessage(problem);
		} finally {
			jobStarting = false;
		}
	}

	function pollJob(): void {
		stopJobTimer();
		jobTimer = setInterval(async () => {
			if (!job) return;
			try {
				job = await service.getJob(job.id);
				if (jobDone) {
					stopJobTimer();
					await load();
				} else if (jobExpired) {
					stopJobTimer();
				}
			} catch (problem) {
				stopJobTimer();
				jobError = jobMessage(problem);
			}
		}, 500);
	}

	function stopJobTimer(): void {
		if (jobTimer) {
			clearInterval(jobTimer);
			jobTimer = null;
		}
	}

	function closeJob(): void {
		stopJobTimer();
		jobOpen = false;
	}

	function jobTone(): StatusTone {
		if (!job) return 'neutral';
		if (job.status === 'succeeded') {
			return job.result_code === 'succeeded_with_warnings' ? 'warning' : 'success';
		}
		if (job.status === 'failed' || job.status === 'expired') return 'failed';
		if (job.status === 'running') return 'running';
		if (job.status === 'sent' || job.status === 'queued') return 'queued';
		return 'neutral';
	}
</script>

{#snippet maintenanceCell(repository: Repository)}
	{#if repository.maintenance_agent_id}
		<span>{maintenanceName(repository)}</span>
	{:else}
		<StatusPill tone="warning">{t('repositories.maintenanceMissing')}</StatusPill>
	{/if}
{/snippet}

{#snippet statusCell(repository: Repository)}
	<StatusPill tone={repository.initialized ? 'success' : 'warning'}>
		{statusLabel(repository)}
	</StatusPill>
{/snippet}

<Page
	title={t('repositories.title')}
	description={t('repositories.description')}
	count={loading ? undefined : repositories.length}
>
	{#snippet actions()}
		{#if canWrite}
			<Button variant="primary" icon="plus" onclick={openCreate}>{t('repositories.add')}</Button>
		{/if}
	{/snippet}

	{#if empty}
		<EmptyState
			icon="repository"
			title={t('repositories.emptyTitle')}
			description={t('repositories.emptyDescription')}
			bordered
		>
			{#if canWrite}
				<Button variant="primary" onclick={openCreate}>{t('repositories.addFirst')}</Button>
			{/if}
		</EmptyState>
	{:else}
		<DataTable
			{columns}
			rows={shown}
			{loading}
			{error}
			rowKey={(repository) => repository.id}
			rowHref={(repository) => resolve('/(app)/repositories/[id]', { id: repository.id })}
			actions={rowActions}
			moreLabel={(repository) => t('repositories.moreActions', { name: repository.name })}
			emptyTitle={t('filter.noMatches')}
			emptyDescription={t('filter.noMatchesDescription')}
		>
			{#snippet toolbar()}
				<FilterBar
					label={t('repositories.filters')}
					value={filters.value}
					search={{
						key: 'q',
						label: t('hosts.search'),
						placeholder: t('repositories.searchPlaceholder')
					}}
					{facets}
					onchange={filters.update}
					onreset={filters.reset}
				/>
			{/snippet}
		</DataTable>
	{/if}

	<div class="note">
		<Alert variant="inline" icon="lock" title={t('repositories.valueNote')} />
	</div>

	<Dialog
		bind:open={unlockOpen}
		size="sm"
		icon="octagon"
		title={t('repositories.unlockAllTitle')}
		onsubmit={submitUnlockAll}
	>
		{#if unlockError}
			<Alert tone="danger" title={unlockError} />
		{/if}
		<Alert tone="danger" icon="alert" title={t('repositories.unlockAllWarningTitle')}>
			{t('repositories.unlockAllWarning', { name: unlockTarget?.name ?? '' })}
		</Alert>
		<TextField
			id="unlock-all-confirm"
			label={t('repositories.unlockAllConfirm')}
			help={t('repositories.unlockAllConfirmHint', { name: unlockTarget?.name ?? '' })}
			autocomplete="off"
			bind:value={unlockConfirm}
		/>
		{#snippet footer()}
			<Button onclick={() => (unlockOpen = false)}>{t('common.cancel')}</Button>
			<Button
				type="submit"
				variant="danger"
				loading={unlocking}
				disabled={!unlockTarget || unlockConfirm !== unlockTarget.name}
			>
				{t('repositories.unlockAll')}
			</Button>
		{/snippet}
	</Dialog>

	<ConfirmDialog
		bind:open={deleteOpen}
		title={t('repositories.deleteTitle')}
		message={t('repositories.deleteBody', { name: deleteTarget?.name ?? '' })}
		confirmLabel={t('repositories.delete')}
		tone="danger"
		loading={deleting}
		onConfirm={confirmDelete}
	/>

	<Dialog
		bind:open={jobOpen}
		size="md"
		title={jobKind === 'init'
			? t('repositories.initializeTitle')
			: t('repositories.testAccessTitle')}
		onsubmit={startJob}
	>
		{#if jobError}
			<Alert tone="danger" title={jobError} />
		{/if}
		{#if jobKind === 'init'}
			<Alert tone="attention" icon="lock" title={t('repositories.initialize')}>
				{t('repositories.initializeBody')}
			</Alert>
		{:else}
			<Alert tone="info" icon="eye" title={t('repositories.testAccess')}>
				{t('repositories.testAccessBody')}
			</Alert>
		{/if}
		{#if job}
			<div class="job">
				<div class="job__row">
					<span class="job__label">{t('repositories.jobStatus')}</span>
					<StatusPill tone={jobTone()}>{t(jobStatusLabels[job.status])}</StatusPill>
				</div>
				<p class="job__result">
					{#if jobExpired}
						{t('repositories.jobExpired')}
					{:else if jobDone && job.result_code}
						{t(jobResultLabels[job.result_code])}
					{:else}
						{t('repositories.jobRunning')}
					{/if}
				</p>
			</div>
		{:else}
			<Select
				id="job-host"
				label={t('repositories.jobHost')}
				help={t('repositories.jobHostHint')}
				options={jobAgentOptions}
				bind:value={jobHost}
			/>
		{/if}
		{#snippet footer()}
			<Button onclick={closeJob}>{t('common.close')}</Button>
			{#if !job}
				<Button type="submit" variant="primary" loading={jobStarting} disabled={!jobHost}>
					{t('repositories.jobStart')}
				</Button>
			{/if}
		{/snippet}
	</Dialog>
</Page>

<style>
	.note {
		margin-top: var(--rh-space-4);
	}

	.job {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-2);
	}

	.job__row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--rh-space-3);
	}

	.job__label {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.job__result {
		margin: 0;
	}
</style>
