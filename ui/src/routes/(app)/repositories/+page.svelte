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
		Dialog,
		FilterBar,
		ListView,
		Select,
		StatusPill,
		TextField
	} from '#lib/components/ui/index.ts';
	import type {
		FilterFacet,
		ListMeta,
		RowAction,
		SelectOption,
		StatusTone
	} from '#lib/components/ui/index.ts';
	import { facetCounts, matchesSearch, splitList } from '#lib/filters.ts';
	import { focusFirstInvalid } from '#lib/form.ts';
	import { ago, plural } from '#lib/list-format.ts';
	import { iconOf, pillOf, repositoryState } from '#lib/vocabulary.ts';
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
	let jobSubmitted = $state(false);
	// The control plane runs an init, and an access test of a local path, only on the maintenance
	// host and refuses another agent (resolve_agent, FR-RP-08).
	const jobHostFixed = $derived(
		jobRepository !== null && (jobKind === 'init' || jobRepository.backend.kind === 'local')
	);
	const jobNoHost = $derived(jobHostFixed && !jobRepository?.maintenance_agent_id);
	const jobHostHref = $derived(
		jobRepository
			? `${resolve('/(app)/repositories/[id]', { id: jobRepository.id })}#repo-agent`
			: ''
	);
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

	function openCreate(): void {
		void goto(resolve('/(app)/repositories/new'));
	}

	const filters = queryFilters(['q', 'backend', 'maintenance', 'state', 'sort', 'order'] as const);

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
			key: 'maintenance',
			label: t('repositories.maintenanceHost'),
			options: facetCounts(repositories, (repository) =>
				repository.maintenance_agent_id ? [repository.maintenance_agent_id] : []
			).map((option) => ({ ...option, label: agentNames.get(option.value) ?? option.value }))
		}
	]);

	const matching = $derived(
		repositories.filter((repository) => {
			const { q, backend, maintenance } = filters.value;
			if (!matchesSearch(q, [repository.name, backendAddress(repository)])) return false;
			if (backend && !splitList(backend).includes(repository.backend.kind)) return false;
			if (maintenance && !splitList(maintenance).includes(repository.maintenance_agent_id ?? '')) {
				return false;
			}
			return true;
		})
	);

	const states = $derived({
		default: 'all',
		options: [
			{ value: 'all', label: t('repositories.tab.all'), count: matching.length },
			{
				value: 'attention',
				label: t('repositories.tab.attention'),
				count: matching.filter((repository) => repositoryState(repository).tone !== 'normal').length
			}
		]
	});

	const sort = $derived({
		default: 'name',
		options: [
			{ value: 'name', label: t('repositories.sort.name') },
			{ value: 'check', label: t('repositories.sort.lastCheck') }
		]
	});

	const shown = $derived.by(() => {
		const list = matching.filter(
			(repository) =>
				filters.value.state !== 'attention' || repositoryState(repository).tone !== 'normal'
		);
		const factor = filters.value.order === 'desc' ? -1 : 1;
		// A repository that was never checked is the oldest.
		const key = (repository: Repository) =>
			filters.value.sort === 'check' ? (repository.last_check?.finished_at ?? '') : repository.name;
		return [...list].sort((a, b) => factor * key(a).localeCompare(key(b)));
	});

	const filtered = $derived(
		repositories.length > 0 &&
			Boolean(
				filters.value.q || filters.value.backend || filters.value.maintenance || filters.value.state
			)
	);

	function resetFilters(): void {
		filters.update({ q: '', backend: '', maintenance: '', state: '' });
	}

	function meta(repository: Repository): ListMeta[] {
		return [
			t(backendLabels[repository.backend.kind]),
			{ text: backendAddress(repository), mono: true },
			repository.last_check
				? {
						text: t('repositories.meta.checked', { time: ago(repository.last_check.finished_at) }),
						time: repository.last_check.finished_at
					}
				: t('repositories.meta.notChecked'),
			repository.maintenance_agent_id
				? t('repositories.meta.maintenance', { name: maintenanceName(repository) })
				: t('repositories.meta.noMaintenance'),
			repository.snapshot_count != null &&
				plural('repositories.snapshotCount', repository.snapshot_count)
		];
	}

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
				icon: 'trash',
				danger: true,
				hidden: !canWrite,
				onSelect: () => {
					deleteTarget = repository;
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
		const fixed = kind === 'init' || repository.backend.kind === 'local';
		jobHost =
			repository.maintenance_agent_id ??
			(fixed ? '' : agents.find((agent) => agent.status === 'active')?.id) ??
			'';
		job = null;
		jobError = null;
		jobStarting = false;
		jobSubmitted = false;
		jobOpen = true;
	}

	async function startJob(): Promise<void> {
		if (!jobRepository || jobNoHost) return;
		jobSubmitted = true;
		if (focusFirstInvalid([['job-host', jobHost === '']])) return;
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

	<ListView
		label={t('repositories.title')}
		items={shown}
		itemKey={(repository) => repository.id}
		icon={(repository) => iconOf(repositoryState(repository))}
		pill={(repository) => pillOf(repositoryState(repository))}
		title={(repository) => repository.name}
		href={(repository) => resolve('/(app)/repositories/[id]', { id: repository.id })}
		{meta}
		actions={rowActions}
		moreLabel={(repository) => t('repositories.moreActions', { name: repository.name })}
		{loading}
		{error}
		{filtered}
		onReset={resetFilters}
		emptyIcon="repository"
		emptyTitle={t('repositories.emptyTitle')}
		emptyDescription={t('repositories.emptyDescription')}
	>
		{#snippet emptyActions()}
			{#if canWrite}
				<Button variant="primary" onclick={openCreate}>{t('repositories.addFirst')}</Button>
			{/if}
		{/snippet}
		{#snippet toolbar()}
			<FilterBar
				label={t('repositories.filters')}
				value={filters.value}
				search={{
					key: 'q',
					label: t('hosts.search'),
					placeholder: t('repositories.searchPlaceholder')
				}}
				{states}
				{facets}
				{sort}
				onchange={filters.update}
				onreset={resetFilters}
			/>
		{/snippet}
	</ListView>

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
		{:else if jobNoHost && jobRepository}
			<Alert tone="attention" icon="alert" title={t('repositories.jobNoHostTitle')}>
				{jobKind === 'init' ? t('repositories.jobNoHostInit') : t('repositories.jobNoHostLocal')}
				<!-- eslint-disable svelte/no-navigation-without-resolve -- resolved path with a hash -->
				<a class="link job__link" href={jobHostHref}>{t('repositories.jobNoHostLink')}</a>
				<!-- eslint-enable svelte/no-navigation-without-resolve -->
			</Alert>
		{:else if jobHostFixed && jobRepository}
			<TextField
				id="job-host"
				label={t('repositories.jobHost')}
				help={jobKind === 'init' ? t('repositories.jobHostInit') : t('repositories.jobHostLocal')}
				value={maintenanceName(jobRepository)}
				disabled
			/>
		{:else}
			<Select
				id="job-host"
				label={t('repositories.jobHost')}
				help={t('repositories.jobHostHint')}
				error={jobSubmitted && jobHost === '' ? t('repositories.jobHostRequired') : undefined}
				options={jobAgentOptions}
				bind:value={jobHost}
			/>
		{/if}
		{#snippet footer()}
			<Button onclick={closeJob}>{t('common.close')}</Button>
			{#if !job}
				<Button
					type="submit"
					variant="primary"
					loading={jobStarting}
					disabled={jobNoHost}
					title={jobNoHost ? t('repositories.jobNoHostTitle') : undefined}
				>
					{t('repositories.jobStart')}
				</Button>
			{/if}
		{/snippet}
	</Dialog>
</Page>

<style>
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

	.job__link {
		display: block;
		margin-top: var(--rh-space-2);
	}

	.job__result {
		margin: 0;
	}
</style>
