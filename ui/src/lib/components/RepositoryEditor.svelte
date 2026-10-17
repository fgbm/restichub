<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { beforeNavigate, goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import type {
		Agent,
		ApiProblem,
		BackendSettings,
		CreateRepositoryRequest,
		Job,
		JobStatus,
		Maintenance,
		Repository,
		RepositoryBackendKind,
		UpdateRepositoryRequest
	} from '#lib/api/index.ts';
	import HostPicker from './HostPicker.svelte';
	import ProfileSection from './ProfileSection.svelte';
	import { RecordPage, SummaryPanel } from './layout/index.ts';
	import { Alert, Button, PasswordField, Select, StatusPill, TextField } from './ui/index.ts';
	import type { KeyValueItem, SelectOption, StatusTone } from './ui/index.ts';
	import { formatLocalTime, formatTimeInZone } from '#lib/format.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import type { SummaryState } from '#lib/profile-form.ts';
	import { browserTimeZone, timeZoneNames } from '#lib/schedule.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	let { repositoryId = null }: { repositoryId?: string | null } = $props();

	type SectionKey = 'basics' | 'location' | 'access' | 'retention' | 'check' | 'host';
	const SECTIONS: SectionKey[] = ['basics', 'location', 'access', 'retention', 'check', 'host'];

	const TIME_ZONES = timeZoneNames();
	const TIME_ZONE_LIST_ID = 'maintenance-time-zones';
	const PREVIEW_RUNS = 3;
	// The defaults of the control plane for a new repository (FR-RP-08, ADR-0034).
	const DEFAULT_RETENTION_CRON = '0 3 * * 0';
	const DEFAULT_CHECK_CRON = '0 4 1 * *';
	const RETENTION_PRESETS: { label: MessageKey; cron: string }[] = [
		{ label: 'repositories.maintenance.preset.daily', cron: '0 3 * * *' },
		{ label: 'repositories.maintenance.preset.weekly', cron: '0 3 * * 0' }
	];
	const CHECK_PRESETS: { label: MessageKey; cron: string }[] = [
		{ label: 'repositories.maintenance.preset.weekly', cron: '0 4 * * 0' },
		{ label: 'repositories.maintenance.preset.monthly', cron: '0 4 1 * *' }
	];

	let loading = $state(true);
	let loadError = $state<string | null>(null);
	let existing = $state<Repository | null>(null);
	let agents = $state<Agent[]>([]);
	let saving = $state(false);
	let saved = false;
	let formError = $state<string | null>(null);
	let formProblem = $state<ApiProblem | null>(null);
	let baseline = $state('');
	let open = $state<Record<string, boolean>>(
		Object.fromEntries(SECTIONS.map((key) => [key, true]))
	);

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
	let restUsername = $state('');
	let restPassword = $state('');
	let maintenanceAgentId = $state('');

	let maintenance = $state<Maintenance | null>(null);
	let retentionCron = $state(DEFAULT_RETENTION_CRON);
	let retentionEnabled = $state(true);
	let checkCron = $state(DEFAULT_CHECK_CRON);
	let checkEnabled = $state(true);
	let maintenanceTimeZone = $state(browserTimeZone());
	let retentionRuns = $state<string[] | null>(null);
	let checkRuns = $state<string[] | null>(null);

	const canWrite = $derived(getSession()?.role === 'admin');
	// The location of an initialized repository cannot change in this version.
	const locationLocked = $derived(existing?.initialized === true);
	// A local path is on the disk of one host, so that host is part of the location (FR-RP-08).
	const isLocal = $derived(kind === 'local');
	const hostLocked = $derived(isLocal && locationLocked);
	const showSchedules = $derived(!existing || maintenance !== null);
	const allOpen = $derived(SECTIONS.every((key) => open[key]));

	const kindOptions = $derived<SelectOption[]>([
		{ value: 's3', label: t('repositories.backendKind.s3') },
		{ value: 'rest', label: t('repositories.backendKind.rest') },
		{ value: 'local', label: t('repositories.backendKind.local') }
	]);

	const hostChoices = $derived(
		agents.filter(
			(agent) =>
				agent.status === 'active' || (hostLocked && agent.id === existing?.maintenance_agent_id)
		)
	);

	const stateKey = $derived(
		JSON.stringify([
			name,
			kind,
			endpoint,
			bucket,
			prefix,
			region,
			path,
			password,
			s3AccessKey,
			s3SecretKey,
			restUsername,
			restPassword,
			maintenanceAgentId,
			retentionCron,
			retentionEnabled,
			checkCron,
			checkEnabled,
			maintenanceTimeZone
		])
	);
	const dirty = $derived(!loading && stateKey !== baseline);

	const hostName = $derived(
		agents.find((agent) => agent.id === maintenanceAgentId)?.hostname ?? ''
	);

	function address(): string {
		switch (kind) {
			case 's3':
				return bucket ? `s3://${bucket}/${prefix}`.replace(/\/$/, '') : '';
			case 'rest':
				return endpoint ? `rest:${endpoint}` : '';
			case 'local':
				return path;
		}
	}

	function accessSummary(): { text: string; state: SummaryState } {
		if (existing) {
			const entered = Boolean(s3AccessKey || s3SecretKey || restPassword);
			return {
				text: t(entered ? 'rp.summary.accessReplace' : 'rp.summary.accessKeep'),
				state: 'on'
			};
		}
		const complete = Boolean(password) && (kind !== 's3' || Boolean(s3AccessKey && s3SecretKey));
		return {
			text: t(complete ? 'rp.summary.accessReady' : 'rp.summary.accessMissing'),
			state: complete ? 'on' : 'warn'
		};
	}

	function scheduleSummary(cron: string, enabled: boolean): { text: string; state: SummaryState } {
		return enabled
			? { text: `${cron} · ${maintenanceTimeZone}`, state: 'on' }
			: { text: t('rp.summary.off'), state: 'off' };
	}

	const summaries = $derived<Record<SectionKey, { text: string; state: SummaryState }>>({
		basics: name ? { text: name, state: 'on' } : { text: t('rp.summary.noName'), state: 'warn' },
		location: address()
			? {
					text: `${t(`repositories.backendKind.${kind}` as MessageKey)} · ${address()}`,
					state: 'on'
				}
			: { text: t('rp.summary.noLocation'), state: 'warn' },
		access: accessSummary(),
		retention: scheduleSummary(retentionCron, retentionEnabled),
		check: scheduleSummary(checkCron, checkEnabled),
		host: hostName
			? { text: hostName, state: 'on' }
			: { text: t('repositories.maintenanceHostNone'), state: 'warn' }
	});

	const titles = $derived<Record<SectionKey, MessageKey>>({
		basics: 'rp.section.basics',
		location: 'rp.section.location',
		access: 'rp.section.access',
		retention: 'repositories.maintenance.retention',
		check: 'repositories.maintenance.check',
		host: isLocal ? 'repositories.repositoryHost' : 'repositories.maintenanceHost'
	});

	const hostHelp = $derived<MessageKey>(
		hostLocked
			? 'repositories.repositoryHostLocked'
			: isLocal
				? 'repositories.repositoryHostHint'
				: 'repositories.maintenanceHostHint'
	);

	const jobStatusLabels: Record<JobStatus, MessageKey> = {
		queued: 'repositories.job.queued',
		sent: 'repositories.job.sent',
		running: 'repositories.job.running',
		succeeded: 'repositories.job.succeeded',
		failed: 'repositories.job.failed',
		expired: 'repositories.job.expired',
		dead_letter: 'repositories.job.failed'
	};

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
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

	function utcTitle(value: string): string {
		const utc = `${formatTimeInZone(value, 'UTC')} UTC`;
		return maintenanceTimeZone === 'UTC'
			? utc
			: `${utc} · ${formatTimeInZone(value, maintenanceTimeZone)} ${maintenanceTimeZone}`;
	}

	async function load(): Promise<void> {
		try {
			const [repositoryList, agentList] = await Promise.all([
				service.listRepositories(),
				service.listAgents()
			]);
			agents = agentList;
			if (repositoryId) {
				const found = repositoryList.find((item) => item.id === repositoryId);
				if (!found) {
					loadError = t('rp.notFound');
					return;
				}
				existing = found;
				name = found.name;
				kind = found.backend.kind;
				if (found.backend.kind === 's3') {
					endpoint = found.backend.endpoint;
					bucket = found.backend.bucket;
					prefix = found.backend.prefix ?? '';
					region = found.backend.region ?? '';
				} else if (found.backend.kind === 'rest') {
					endpoint = found.backend.endpoint;
					restUsername = found.backend.username ?? '';
				} else {
					path = found.backend.path;
				}
				maintenanceAgentId = found.maintenance_agent_id ?? '';
				await loadMaintenance(found);
			}
			baseline = stateKey;
		} catch (problem) {
			loadError = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	async function loadMaintenance(repository: Repository): Promise<void> {
		try {
			const loaded = await service.getMaintenance(repository.id);
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

	onMount(async () => {
		await load();
		await tick();
		// The init dialog links to #repo-agent when the repository has no maintenance host.
		const id = window.location.hash.slice(1);
		if (id) document.getElementById(id)?.focus();
	});

	const resultItems = $derived<KeyValueItem[]>(
		SECTIONS.filter((key) => showSchedules || (key !== 'retention' && key !== 'check')).map(
			(key) => ({ key, label: t(titles[key]), value: summaries[key].text })
		)
	);

	beforeNavigate(({ cancel }) => {
		if (dirty && !saved && !window.confirm(t('rp.unsaved'))) cancel();
	});

	// The page previews the next fire times of each schedule from the form, so the user sees them
	// before the save (FR-SC-01). The list follows the edit of the cron or the time zone.
	$effect(() => {
		if (loading) return;
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

	// A section that holds a server error opens, so the message is not hidden.
	$effect(() => {
		const field = formProblem?.field ?? '';
		const target: [string, SectionKey][] = [
			['name', 'basics'],
			['backend.username', 'access'],
			['backend', 'location'],
			['password', 'access'],
			['s3_', 'access'],
			['rest_password', 'access'],
			['time_zone', 'retention'],
			['retention', 'retention'],
			['check', 'check'],
			['maintenance_agent_id', 'host']
		];
		const hit = target.find(([prefix]) => field.startsWith(prefix));
		if (hit) open[hit[1]] = true;
	});

	function setAll(value: boolean): void {
		for (const key of SECTIONS) open[key] = value;
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
				return { kind: 'rest', endpoint, username: restUsername.trim() || null };
			case 'local':
				return { kind: 'local', path };
		}
	}

	async function save(): Promise<void> {
		saving = true;
		formError = null;
		formProblem = null;
		try {
			if (existing) {
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
				await service.updateRepository(existing.id, request);
				if (maintenance) {
					await service.updateMaintenance(existing.id, {
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
				const created = await service.createRepository(request);
				pushToast('success', t('repositories.created'));
				try {
					await service.updateMaintenance(created.id, {
						time_zone: maintenanceTimeZone,
						retention: { cron: retentionCron, enabled: retentionEnabled },
						check: { cron: checkCron, enabled: checkEnabled }
					});
				} catch (problem) {
					pushToast('danger', messageOf(problem));
				}
			}
			saved = true;
			await goto(resolve('/(app)/repositories'));
		} catch (problem) {
			formProblem = problem instanceof ApiError ? problem.problem : null;
			formError = formProblem ? null : messageOf(problem);
		} finally {
			saving = false;
		}
	}
</script>

{#snippet runsList(runs: string[] | null)}
	{#if runs && runs.length > 0}
		<ul class="runs">
			{#each runs as run (run)}
				<li title={utcTitle(run)}>{formatLocalTime(run)}</li>
			{/each}
		</ul>
	{:else}
		<p class="muted">{t('repositories.maintenance.nextRunsEmpty')}</p>
	{/if}
{/snippet}

{#snippet lastRun(job: Job | null | undefined)}
	{#if job}
		<span class="last">
			<StatusPill tone={lastTone(job)}>{t(jobStatusLabels[job.status])}</StatusPill>
			<span class="muted" title={utcTitle(job.finished_at ?? job.created_at)}>
				{formatLocalTime(job.finished_at ?? job.created_at)}
			</span>
		</span>
	{:else}
		<span class="muted">{t('repositories.maintenance.lastNone')}</span>
	{/if}
{/snippet}

<RecordPage
	title={repositoryId ? (existing?.name ?? '') : t('rp.newTitle')}
	description={t(repositoryId ? 'repositories.editDescription' : 'repositories.addDescription')}
	{loading}
	error={loadError}
	onsubmit={() => void save()}
>
	{#snippet actions()}
		<Button size="sm" onclick={() => setAll(true)} disabled={allOpen}>{t('pe.expandAll')}</Button>
		<Button size="sm" onclick={() => setAll(false)}>{t('pe.collapseAll')}</Button>
	{/snippet}
	{#snippet banners()}
		{#if formError}
			<Alert tone="danger" title={formError} />
		{/if}
	{/snippet}
	{#snippet side()}
		<SummaryPanel title={t('rp.side.result')} items={resultItems} />
	{/snippet}
	{#snippet footer()}
		<div class="savebar">
			<span class="muted savebar__text">
				{#if !canWrite}
					{t('rp.save.readonly')}
				{:else if dirty}
					{t('rp.save.dirty')}
				{/if}
			</span>
			<Button onclick={() => goto(resolve('/(app)/repositories'))}>{t('common.cancel')}</Button>
			{#if canWrite}
				<Button type="submit" variant="primary" loading={saving}>{t('common.save')}</Button>
			{/if}
		</div>
	{/snippet}

	<ProfileSection
		id="basics"
		title={t(titles.basics)}
		summary={summaries.basics.text}
		state={summaries.basics.state}
		required
		bind:open={open.basics}
	>
		<TextField
			id="repo-name"
			label={t('repositories.repository')}
			error={fieldError(formProblem, 'name')}
			bind:value={name}
			required
		/>
	</ProfileSection>

	<ProfileSection
		id="location"
		title={t(titles.location)}
		summary={summaries.location.text}
		state={summaries.location.state}
		required
		bind:open={open.location}
	>
		{#if locationLocked}
			<Alert tone="attention" icon="lock" title={t('repositories.locationLocked')} />
		{/if}
		<Select
			id="repo-kind"
			label={t('repositories.backendKind')}
			options={kindOptions}
			error={fieldError(formProblem, 'backend')}
			bind:value={kind}
			disabled={!!existing}
		/>
		{#if kind === 's3'}
			<div class="grid2">
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
			</div>
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
	</ProfileSection>

	<ProfileSection
		id="access"
		title={t(titles.access)}
		summary={summaries.access.text}
		state={summaries.access.state}
		required
		bind:open={open.access}
	>
		{#if existing}
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
			<div class="grid2">
				<PasswordField
					id="repo-s3-access-key"
					label={t('repositories.s3AccessKey')}
					help={t('repositories.secretHint')}
					error={fieldError(formProblem, 's3_access_key')}
					autocomplete="new-password"
					bind:value={s3AccessKey}
					required={!existing}
				/>
				<PasswordField
					id="repo-s3-secret-key"
					label={t('repositories.s3SecretKey')}
					help={t('repositories.secretHint')}
					error={fieldError(formProblem, 's3_secret_key')}
					autocomplete="new-password"
					bind:value={s3SecretKey}
					required={!existing}
				/>
			</div>
		{:else if kind === 'rest'}
			<TextField
				id="repo-rest-username"
				label={t('repositories.restUsername')}
				help={t('repositories.restUsernameHint')}
				error={fieldError(formProblem, 'backend.username')}
				autocomplete="off"
				bind:value={restUsername}
			/>
			<PasswordField
				id="repo-rest-password"
				label={t('repositories.restPassword')}
				help={t('repositories.restPasswordHint')}
				error={fieldError(formProblem, 'rest_password')}
				autocomplete="new-password"
				bind:value={restPassword}
			/>
		{/if}
		<a
			class="manage-secrets"
			href={resolve('/(app)/settings/access/secrets')}
			target="_blank"
			rel="noopener"
		>
			{t('secrets.manage')}
		</a>
	</ProfileSection>

	{#if showSchedules}
		<ProfileSection
			id="retention"
			title={t(titles.retention)}
			summary={summaries.retention.text}
			state={summaries.retention.state}
			switchable
			bind:enabled={retentionEnabled}
			bind:open={open.retention}
		>
			<p class="lead">{t('repositories.maintenance.description')}</p>
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
			<p class="lead">{t('repositories.maintenance.retentionHint')}</p>
			<TextField
				id="maintenance-retention-cron"
				mono
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
			<div class="runs-block">
				<span class="muted">{t('repositories.maintenance.nextRuns')}</span>
				{@render runsList(retentionEnabled ? retentionRuns : null)}
			</div>
			{#if maintenance}
				<div class="runs-block">
					<span class="muted">{t('repositories.maintenance.last')}</span>
					{@render lastRun(maintenance.last_retention)}
				</div>
			{/if}
		</ProfileSection>

		<ProfileSection
			id="check"
			title={t(titles.check)}
			summary={summaries.check.text}
			state={summaries.check.state}
			switchable
			bind:enabled={checkEnabled}
			bind:open={open.check}
		>
			<p class="lead">{t('repositories.maintenance.checkHint')}</p>
			<TextField
				id="maintenance-check-cron"
				mono
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
			<p class="muted">{t('rp.check.zone', { zone: maintenanceTimeZone })}</p>
			<div class="runs-block">
				<span class="muted">{t('repositories.maintenance.nextRuns')}</span>
				{@render runsList(checkEnabled ? checkRuns : null)}
			</div>
			{#if maintenance}
				<div class="runs-block">
					<span class="muted">{t('repositories.maintenance.last')}</span>
					{@render lastRun(maintenance.last_check)}
				</div>
			{/if}
		</ProfileSection>
	{/if}

	<ProfileSection
		id="host"
		title={t(titles.host)}
		summary={summaries.host.text}
		state={summaries.host.state}
		required={isLocal}
		bind:open={open.host}
	>
		<HostPicker
			id="repo-agent"
			label={t(titles.host)}
			help={t(hostHelp)}
			hosts={hostChoices}
			allLabel={isLocal ? undefined : t('repositories.maintenanceHostNone')}
			placeholder={isLocal ? t('repositories.hostPlaceholder') : undefined}
			error={fieldError(formProblem, 'maintenance_agent_id')}
			bind:value={maintenanceAgentId}
			disabled={hostLocked}
			required={isLocal}
		/>
		{#if showSchedules && !maintenanceAgentId}
			<Alert tone="attention" icon="alert" title={t('repositories.maintenance.noHostTitle')}>
				{t('repositories.maintenance.noHostBody')}
			</Alert>
		{/if}
	</ProfileSection>
</RecordPage>

<style>
	.muted {
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.lead {
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.grid2 {
		display: grid;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		gap: var(--rh-space-3);
	}

	.manage-secrets {
		align-self: flex-start;
		font-size: var(--rh-font-size-sm);
	}

	.presets {
		display: flex;
		flex-wrap: wrap;
		gap: var(--rh-space-2);
	}

	.runs-block {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-1);
	}

	.runs {
		display: flex;
		flex-direction: column;
		gap: 2px;
		margin: 0;
		padding: 0;
		list-style: none;
		font-size: var(--rh-font-size-sm);
	}

	.runs li {
		width: fit-content;
		cursor: help;
	}

	.last {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.savebar {
		display: flex;
		align-items: center;
		gap: var(--rh-space-3);
		padding: var(--rh-space-3) var(--rh-space-4);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-lg);
		background-color: var(--rh-color-surface);
	}

	.savebar__text {
		flex: 1;
	}

	@media (max-width: 720px) {
		.grid2 {
			grid-template-columns: minmax(0, 1fr);
		}
	}
</style>
