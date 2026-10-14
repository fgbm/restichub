<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import type {
		ApiProblem,
		BackendSettings,
		CreateRepositoryRequest,
		Job,
		JobResultCode,
		JobStatus,
		Maintenance,
		MaintenanceTask,
		Repository,
		RepositoryBackendKind,
		UpdateRepositoryRequest
	} from '#lib/api/index.ts';
	import {
		Alert,
		Button,
		Checkbox,
		ConfirmDialog,
		Dialog,
		DropdownMenu,
		EmptyState,
		PageHeader,
		PasswordField,
		Select,
		StatusPill,
		Table,
		TextField
	} from '#lib/components/ui/index.ts';
	import type { SelectOption, StatusTone, TableColumn } from '#lib/components/ui/index.ts';
	import { t, tErrorCode, type MessageKey } from '#lib/i18n/index.ts';
	import { formatTimeInZoneWithLocal } from '#lib/format.ts';
	import { timeZoneNames } from '#lib/schedule.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	let repositories = $state<Repository[]>([]);
	let agents = $state<{ id: string; hostname: string; status: string }[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let formOpen = $state(false);
	let editing = $state<Repository | null>(null);
	let saving = $state(false);
	let formError = $state<string | null>(null);
	let formProblem = $state<ApiProblem | null>(null);

	let name = $state('');
	let kind = $state<RepositoryBackendKind>('s3');
	let endpoint = $state('');
	let bucket = $state('');
	let prefix = $state('');
	let region = $state('');
	let path = $state('');
	let password = $state('');
	let s3AccessKey = $state('');
	let s3SecretKey = $state('');
	let restPassword = $state('');
	let maintenanceAgentId = $state('');

	// The maintenance schedules of the repository in the dialog (FR-RP-08).
	const TIME_ZONES = timeZoneNames();
	const TIME_ZONE_LIST_ID = 'maintenance-time-zones';
	const PREVIEW_RUNS = 3;
	const RETENTION_PRESETS: { label: MessageKey; cron: string }[] = [
		{ label: 'repositories.maintenance.preset.daily', cron: '0 3 * * *' },
		{ label: 'repositories.maintenance.preset.weekly', cron: '0 3 * * 0' }
	];
	const CHECK_PRESETS: { label: MessageKey; cron: string }[] = [
		{ label: 'repositories.maintenance.preset.weekly', cron: '0 4 * * 0' },
		{ label: 'repositories.maintenance.preset.monthly', cron: '0 4 1 * *' }
	];
	let maintenance = $state<Maintenance | null>(null);
	let retentionCron = $state('');
	let retentionEnabled = $state(true);
	let checkCron = $state('');
	let checkEnabled = $state(true);
	let maintenanceTimeZone = $state('UTC');
	let retentionRuns = $state<string[] | null>(null);
	let checkRuns = $state<string[] | null>(null);

	let deleteTarget = $state<Repository | null>(null);
	let deleteOpen = $state(false);
	let deleting = $state(false);

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
		repository_locked: 'repositories.result.repository_locked'
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

	// The location of an initialized repository cannot change in this version (MEDIUM 4).
	const locationLocked = $derived(editing?.initialized === true);

	const canWrite = $derived(getSession()?.role === 'admin');
	const canRun = $derived(getSession()?.role === 'admin' || getSession()?.role === 'operator');
	const agentNames = $derived(new Map(agents.map((agent) => [agent.id, agent.hostname])));

	const backendLabels: Record<RepositoryBackendKind, MessageKey> = {
		s3: 'repositories.backendKind.s3',
		rest: 'repositories.backendKind.rest',
		local: 'repositories.backendKind.local'
	};

	const kindOptions = $derived<SelectOption[]>([
		{ value: 's3', label: t('repositories.backendKind.s3') },
		{ value: 'rest', label: t('repositories.backendKind.rest') },
		{ value: 'local', label: t('repositories.backendKind.local') }
	]);

	const agentOptions = $derived<SelectOption[]>([
		{ value: '', label: t('repositories.maintenanceHostNone') },
		...agents
			.filter((agent) => agent.status === 'active')
			.map((agent) => ({ value: agent.id, label: agent.hostname }))
	]);

	const columns = $derived<TableColumn<Repository>[]>([
		{
			key: 'name',
			header: t('repositories.repository'),
			value: (row) => row.name,
			render: nameCell
		},
		{
			key: 'backend',
			header: t('repositories.backend'),
			value: (row) => t(backendLabels[row.backend.kind]),
			render: backendCell
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

	// A closed dialog drops the secret values, so they never stay in the component state.
	$effect(() => {
		if (!formOpen) resetForm();
	});

	// The dialog previews the next fire times of each schedule from the form, so the user sees them
	// before the save (FR-SC-01). The list follows the edit of the cron or the time zone.
	$effect(() => {
		if (!formOpen || !editing) return;
		const zone = maintenanceTimeZone;
		const retention = retentionCron;
		const check = checkCron;
		const handle = setTimeout(() => {
			void loadRuns(retention, zone).then((runs) => (retentionRuns = runs));
			void loadRuns(check, zone).then((runs) => (checkRuns = runs));
		}, 200);
		return () => clearTimeout(handle);
	});

	async function loadRuns(cron: string, zone: string): Promise<string[] | null> {
		if (!cron.trim() || !zone.trim()) return null;
		try {
			const response = await service.previewSchedule({
				cron,
				time_zone: zone,
				count: PREVIEW_RUNS
			});
			return response.runs;
		} catch {
			return null;
		}
	}

	async function loadMaintenance(repository: Repository): Promise<void> {
		try {
			const loaded = await service.getMaintenance(repository.id);
			// The dialog may have closed or moved to another repository while the call ran.
			if (editing?.id !== repository.id) return;
			maintenance = loaded;
			retentionCron = loaded.retention.cron;
			retentionEnabled = loaded.retention.enabled;
			checkCron = loaded.check.cron;
			checkEnabled = loaded.check.enabled;
			maintenanceTimeZone = loaded.retention.time_zone;
		} catch (problem) {
			formError = messageOf(problem);
		}
	}

	function resetForm(): void {
		maintenance = null;
		retentionCron = '';
		retentionEnabled = true;
		checkCron = '';
		checkEnabled = true;
		maintenanceTimeZone = 'UTC';
		retentionRuns = null;
		checkRuns = null;
		name = '';
		kind = 's3';
		endpoint = '';
		bucket = '';
		prefix = '';
		region = '';
		path = '';
		password = '';
		s3AccessKey = '';
		s3SecretKey = '';
		restPassword = '';
		maintenanceAgentId = '';
		formError = null;
		formProblem = null;
	}

	function openCreate(): void {
		editing = null;
		resetForm();
		formOpen = true;
	}

	function openEdit(repository: Repository): void {
		editing = repository;
		resetForm();
		name = repository.name;
		kind = repository.backend.kind;
		if (repository.backend.kind === 's3') {
			endpoint = repository.backend.endpoint;
			bucket = repository.backend.bucket;
			prefix = repository.backend.prefix ?? '';
			region = repository.backend.region ?? '';
		} else if (repository.backend.kind === 'rest') {
			endpoint = repository.backend.endpoint;
		} else {
			path = repository.backend.path;
		}
		maintenanceAgentId = repository.maintenance_agent_id ?? '';
		formOpen = true;
		void loadMaintenance(repository);
	}

	function backendRequest(): BackendSettings {
		switch (kind) {
			case 's3':
				return {
					kind: 's3',
					endpoint,
					bucket,
					prefix: prefix || undefined,
					region: region || undefined
				};
			case 'rest':
				return { kind: 'rest', endpoint };
			case 'local':
				return { kind: 'local', path };
		}
	}

	async function save(): Promise<void> {
		saving = true;
		formError = null;
		formProblem = null;
		try {
			if (editing) {
				const request: UpdateRepositoryRequest = {
					name,
					backend: backendRequest(),
					maintenance_agent_id: maintenanceAgentId
				};
				if (kind === 's3') {
					if (s3AccessKey) request.s3_access_key = s3AccessKey;
					if (s3SecretKey) request.s3_secret_key = s3SecretKey;
				}
				if (kind === 'rest' && restPassword) request.rest_password = restPassword;
				await service.updateRepository(editing.id, request);
				if (maintenance) {
					await service.updateMaintenance(editing.id, {
						time_zone: maintenanceTimeZone,
						retention: { cron: retentionCron, enabled: retentionEnabled },
						check: { cron: checkCron, enabled: checkEnabled }
					});
				}
				pushToast('success', t('repositories.updated'));
			} else {
				const request: CreateRepositoryRequest = {
					name,
					backend: backendRequest(),
					password,
					maintenance_agent_id: maintenanceAgentId || undefined
				};
				if (kind === 's3') {
					request.s3_access_key = s3AccessKey;
					request.s3_secret_key = s3SecretKey;
				}
				if (kind === 'rest' && restPassword) request.rest_password = restPassword;
				await service.createRepository(request);
				pushToast('success', t('repositories.created'));
			}
			formOpen = false;
			resetForm();
			await load();
		} catch (problem) {
			formProblem = problem instanceof ApiError ? problem.problem : null;
			formError = formProblem ? null : messageOf(problem);
		} finally {
			saving = false;
		}
	}

	async function runTask(repository: Repository, task: MaintenanceTask): Promise<void> {
		try {
			await service.runMaintenance(repository.id, task);
			pushToast('success', t('repositories.maintenanceQueued'));
		} catch (problem) {
			pushToast('danger', messageOf(problem));
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

	function lastTone(entry: Job): StatusTone {
		if (entry.status === 'succeeded') {
			return entry.result_code === 'succeeded_with_warnings' ? 'warning' : 'success';
		}
		if (entry.status === 'failed' || entry.status === 'expired' || entry.status === 'dead_letter') {
			return 'failed';
		}
		if (entry.status === 'running') return 'running';
		return 'queued';
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

{#snippet nameCell(repository: Repository)}
	<span class="repo">
		<span class="repo__name">{repository.name}</span>
		<span class="repo__meta">{backendAddress(repository)}</span>
	</span>
{/snippet}

{#snippet backendCell(repository: Repository)}
	<span class="repo__backend">{t(backendLabels[repository.backend.kind])}</span>
{/snippet}

{#snippet maintenanceCell(repository: Repository)}
	{#if repository.maintenance_agent_id}
		<span>{maintenanceName(repository)}</span>
	{:else}
		<StatusPill tone="warning">{t('repositories.maintenanceMissing')}</StatusPill>
	{/if}
{/snippet}

{#snippet lastRun(job: Job | null | undefined)}
	{#if job}
		<span class="last">
			<StatusPill tone={lastTone(job)}>{t(jobStatusLabels[job.status])}</StatusPill>
			<span class="last__time">
				{formatTimeInZoneWithLocal(job.finished_at ?? job.created_at, maintenanceTimeZone)}
			</span>
		</span>
	{:else}
		<span class="last__time">{t('repositories.maintenance.lastNone')}</span>
	{/if}
{/snippet}

{#snippet runsList(runs: string[] | null)}
	{#if runs && runs.length > 0}
		<ul class="runs">
			{#each runs as run (run)}
				<li>{formatTimeInZoneWithLocal(run, maintenanceTimeZone)}</li>
			{/each}
		</ul>
	{:else}
		<p class="hint">{t('repositories.maintenance.nextRunsEmpty')}</p>
	{/if}
{/snippet}

{#snippet statusCell(repository: Repository)}
	<StatusPill tone={repository.initialized ? 'success' : 'warning'}>
		{statusLabel(repository)}
	</StatusPill>
{/snippet}

<svelte:head>
	<title>{t('repositories.title')} · {t('app.name')}</title>
</svelte:head>

<PageHeader
	title={t('repositories.title')}
	description={t('repositories.description')}
	count={loading ? undefined : repositories.length}
>
	{#if canWrite}
		<Button variant="primary" icon="plus" onclick={openCreate}>{t('repositories.add')}</Button>
	{/if}
</PageHeader>

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
	<Table
		{columns}
		rows={repositories}
		{loading}
		{error}
		rowKey={(repository) => repository.id}
		filterPlaceholder={t('repositories.searchPlaceholder')}
	>
		{#snippet rowActions(repository)}
			{#if canWrite || canRun}
				<DropdownMenu
					icon="more"
					align="end"
					label={t('repositories.moreActions', { name: repository.name })}
					items={[
						...(canRun && repository.maintenance_agent_id && repository.initialized
							? [
									{
										label: t('repositories.runRetention'),
										icon: 'refresh' as const,
										onSelect: () => void runTask(repository, 'retention')
									},
									{
										label: t('repositories.runCheck'),
										icon: 'refresh' as const,
										onSelect: () => void runTask(repository, 'check')
									}
								]
							: []),
						...(canWrite
							? [
									...(repository.initialized
										? []
										: [
												{
													label: t('repositories.initialize'),
													icon: 'plus' as const,
													onSelect: () => openJob(repository, 'init')
												}
											]),
									{
										label: t('repositories.testAccess'),
										icon: 'eye' as const,
										onSelect: () => openJob(repository, 'test_access')
									},
									{
										label: t('repositories.edit'),
										icon: 'settings' as const,
										onSelect: () => openEdit(repository)
									},
									{
										label: t('repositories.delete'),
										icon: 'octagon' as const,
										tone: 'danger' as const,
										onSelect: () => {
											deleteTarget = repository;
											deleteOpen = true;
										}
									}
								]
							: [])
					]}
				/>
			{/if}
		{/snippet}
	</Table>
{/if}

<div class="note">
	<Alert variant="inline" icon="lock" title={t('repositories.valueNote')} />
</div>

<Dialog
	bind:open={formOpen}
	size="lg"
	title={editing ? t('repositories.editTitle') : t('repositories.addTitle')}
	description={editing ? t('repositories.editDescription') : t('repositories.addDescription')}
	onsubmit={save}
>
	{#if formError}
		<Alert tone="danger" title={formError} />
	{/if}
	{#if locationLocked}
		<Alert tone="attention" icon="lock" title={t('repositories.locationLocked')} />
	{/if}
	<div class="form">
		<TextField
			id="repo-name"
			label={t('repositories.repository')}
			error={fieldError(formProblem, 'name')}
			bind:value={name}
			required
		/>
		<Select
			id="repo-kind"
			label={t('repositories.backendKind')}
			options={kindOptions}
			error={fieldError(formProblem, 'backend')}
			bind:value={kind}
			disabled={!!editing}
		/>
		{#if kind === 's3'}
			<TextField
				id="repo-endpoint"
				label={t('repositories.endpoint')}
				placeholder={t('repositories.endpointPlaceholder')}
				error={fieldError(formProblem, 'backend.endpoint')}
				bind:value={endpoint}
				disabled={locationLocked}
				required
			/>
			<TextField
				id="repo-bucket"
				label={t('repositories.bucket')}
				placeholder={t('repositories.bucketPlaceholder')}
				error={fieldError(formProblem, 'backend.bucket')}
				bind:value={bucket}
				disabled={locationLocked}
				required
			/>
			<TextField
				id="repo-prefix"
				label={t('repositories.prefix')}
				placeholder={t('repositories.prefixPlaceholder')}
				error={fieldError(formProblem, 'backend.prefix')}
				bind:value={prefix}
				disabled={locationLocked}
			/>
			<TextField
				id="repo-region"
				label={t('repositories.region')}
				placeholder={t('repositories.regionPlaceholder')}
				error={fieldError(formProblem, 'backend.region')}
				bind:value={region}
				disabled={locationLocked}
			/>
		{:else if kind === 'rest'}
			<TextField
				id="repo-endpoint"
				label={t('repositories.endpoint')}
				placeholder={t('repositories.endpointPlaceholder')}
				error={fieldError(formProblem, 'backend.endpoint')}
				bind:value={endpoint}
				disabled={locationLocked}
				required
			/>
		{:else}
			<TextField
				id="repo-path"
				label={t('repositories.path')}
				placeholder={t('repositories.pathPlaceholder')}
				error={fieldError(formProblem, 'backend.path')}
				bind:value={path}
				disabled={locationLocked}
				required
			/>
		{/if}

		{#if editing}
			<Alert tone="info" icon="lock" title={t('repositories.password')}>
				{t('repositories.passwordLocked')}
			</Alert>
		{:else}
			<PasswordField
				id="repo-password"
				label={t('repositories.password')}
				help={t('repositories.passwordHint')}
				error={fieldError(formProblem, 'password')}
				autocomplete="new-password"
				bind:value={password}
				required
			/>
		{/if}
		{#if kind === 's3'}
			<PasswordField
				id="repo-s3-access-key"
				label={t('repositories.s3AccessKey')}
				help={t('repositories.secretHint')}
				error={fieldError(formProblem, 's3_access_key')}
				autocomplete="new-password"
				bind:value={s3AccessKey}
				required={!editing}
			/>
			<PasswordField
				id="repo-s3-secret-key"
				label={t('repositories.s3SecretKey')}
				help={t('repositories.secretHint')}
				error={fieldError(formProblem, 's3_secret_key')}
				autocomplete="new-password"
				bind:value={s3SecretKey}
				required={!editing}
			/>
		{:else if kind === 'rest'}
			<PasswordField
				id="repo-rest-password"
				label={t('repositories.restPassword')}
				help={t('repositories.restPasswordHint')}
				error={fieldError(formProblem, 'rest_password')}
				autocomplete="new-password"
				bind:value={restPassword}
			/>
		{/if}

		<Select
			id="repo-agent"
			label={t('repositories.maintenanceHost')}
			help={t('repositories.maintenanceHostHint')}
			options={agentOptions}
			error={fieldError(formProblem, 'maintenance_agent_id')}
			bind:value={maintenanceAgentId}
		/>

		{#if editing && maintenance}
			<section class="section" aria-labelledby="maintenance-title">
				<h3 class="section__title" id="maintenance-title">{t('repositories.maintenance.title')}</h3>
				<p class="hint">{t('repositories.maintenance.description')}</p>
				{#if !maintenanceAgentId}
					<Alert tone="attention" icon="alert" title={t('repositories.maintenance.noHostTitle')}>
						{t('repositories.maintenance.noHostBody')}
					</Alert>
				{/if}
				<TextField
					id="maintenance-time-zone"
					label={t('repositories.maintenance.timeZone')}
					help={t('repositories.maintenance.timeZoneHint')}
					error={fieldError(formProblem, 'time_zone')}
					list={TIME_ZONE_LIST_ID}
					bind:value={maintenanceTimeZone}
					required
				/>
				<datalist id={TIME_ZONE_LIST_ID}>
					{#each TIME_ZONES as zone (zone)}
						<option value={zone}></option>
					{/each}
				</datalist>

				<div class="task">
					<h4 class="task__title">{t('repositories.maintenance.retention')}</h4>
					<p class="hint">{t('repositories.maintenance.retentionHint')}</p>
					<TextField
						id="maintenance-retention-cron"
						label={t('repositories.maintenance.cron')}
						help={t('repositories.maintenance.cronHint')}
						error={fieldError(formProblem, 'retention.cron')}
						bind:value={retentionCron}
						required
					/>
					<div class="presets" role="group" aria-label={t('repositories.maintenance.presets')}>
						{#each RETENTION_PRESETS as preset (preset.cron)}
							<Button type="button" size="sm" onclick={() => (retentionCron = preset.cron)}>
								{t(preset.label)}
							</Button>
						{/each}
					</div>
					<Checkbox label={t('repositories.maintenance.enabled')} bind:checked={retentionEnabled} />
					<div class="task__runs">
						<span class="task__label">{t('repositories.maintenance.nextRuns')}</span>
						{@render runsList(retentionEnabled ? retentionRuns : null)}
					</div>
					<div class="task__runs">
						<span class="task__label">{t('repositories.maintenance.last')}</span>
						{@render lastRun(maintenance.last_retention)}
					</div>
				</div>

				<div class="task">
					<h4 class="task__title">{t('repositories.maintenance.check')}</h4>
					<p class="hint">{t('repositories.maintenance.checkHint')}</p>
					<TextField
						id="maintenance-check-cron"
						label={t('repositories.maintenance.cron')}
						help={t('repositories.maintenance.cronHint')}
						error={fieldError(formProblem, 'check.cron')}
						bind:value={checkCron}
						required
					/>
					<div class="presets" role="group" aria-label={t('repositories.maintenance.presets')}>
						{#each CHECK_PRESETS as preset (preset.cron)}
							<Button type="button" size="sm" onclick={() => (checkCron = preset.cron)}>
								{t(preset.label)}
							</Button>
						{/each}
					</div>
					<Checkbox label={t('repositories.maintenance.enabled')} bind:checked={checkEnabled} />
					<div class="task__runs">
						<span class="task__label">{t('repositories.maintenance.nextRuns')}</span>
						{@render runsList(checkEnabled ? checkRuns : null)}
					</div>
					<div class="task__runs">
						<span class="task__label">{t('repositories.maintenance.last')}</span>
						{@render lastRun(maintenance.last_check)}
					</div>
				</div>
			</section>
		{/if}
	</div>
	{#snippet footer()}
		<Button onclick={() => (formOpen = false)}>{t('common.cancel')}</Button>
		<Button type="submit" variant="primary" loading={saving}>
			{t('common.save')}
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
	title={jobKind === 'init' ? t('repositories.initializeTitle') : t('repositories.testAccessTitle')}
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

<style>
	.repo {
		display: flex;
		flex-direction: column;
	}

	.repo__name {
		font-weight: var(--rh-font-weight-semibold);
		white-space: nowrap;
	}

	.repo__meta {
		color: var(--rh-color-text-muted);
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-sm);
	}

	.repo__backend {
		white-space: nowrap;
	}

	.form {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-4);
	}

	.note {
		margin-top: var(--rh-space-4);
	}

	.section {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
	}

	.section__title {
		margin: 0;
		padding-bottom: var(--rh-space-1);
		border-bottom: var(--rh-border-width) solid var(--rh-color-border);
		font-size: var(--rh-font-size-sm);
		font-weight: var(--rh-font-weight-semibold);
		text-transform: uppercase;
		letter-spacing: 0.04em;
		color: var(--rh-color-text-muted);
	}

	.task {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-2);
		padding: var(--rh-space-3);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
	}

	.task__title {
		margin: 0;
		font-size: var(--rh-font-size-md);
		font-weight: var(--rh-font-weight-semibold);
	}

	.task__runs {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-1);
	}

	.task__label {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.presets {
		display: flex;
		flex-wrap: wrap;
		gap: var(--rh-space-2);
	}

	.runs {
		margin: 0;
		padding-left: var(--rh-space-4);
	}

	.hint {
		margin: 0;
		color: var(--rh-color-text-muted);
	}

	.last {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.last__time {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
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
