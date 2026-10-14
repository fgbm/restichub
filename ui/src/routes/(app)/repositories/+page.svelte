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
		Repository,
		RepositoryBackendKind,
		UpdateRepositoryRequest
	} from '#lib/api/index.ts';
	import {
		Alert,
		Button,
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

	function resetForm(): void {
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
	<span>{maintenanceName(repository)}</span>
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
			{#if canWrite}
				<DropdownMenu
					icon="more"
					align="end"
					label={t('repositories.moreActions', { name: repository.name })}
					items={[
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
							icon: 'eye',
							onSelect: () => openJob(repository, 'test_access')
						},
						{
							label: t('repositories.edit'),
							icon: 'settings',
							onSelect: () => openEdit(repository)
						},
						{
							label: t('repositories.delete'),
							icon: 'octagon',
							tone: 'danger',
							onSelect: () => {
								deleteTarget = repository;
								deleteOpen = true;
							}
						}
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
