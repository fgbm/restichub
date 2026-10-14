<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import type {
		ApiProblem,
		CreateProfileRequest,
		HookPhase,
		HookRef,
		PreviewResponse,
		Profile,
		UpdateProfileRequest
	} from '#lib/api/index.ts';
	import {
		Alert,
		Badge,
		Button,
		Checkbox,
		ConfirmDialog,
		Dialog,
		DropdownMenu,
		EmptyState,
		Field,
		PageHeader,
		Select,
		SidePanel,
		Table,
		TextField
	} from '#lib/components/ui/index.ts';
	import type { MenuItem, SelectOption, TableColumn } from '#lib/components/ui/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	type HookDraft = {
		hookId: string;
		phase: HookPhase;
		parameters: { name: string; value: string }[];
	};

	const PHASES: HookPhase[] = ['before', 'after', 'on_success', 'on_error', 'finally'];

	let profiles = $state<Profile[]>([]);
	let repositories = $state<{ id: string; name: string }[]>([]);
	let agents = $state<{ id: string; hostname: string; status: string }[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let formOpen = $state(false);
	let editing = $state<Profile | null>(null);
	let saving = $state(false);
	let formError = $state<string | null>(null);
	let formProblem = $state<ApiProblem | null>(null);

	let name = $state('');
	let repositoryId = $state('');
	let pathsAll = $state('');
	let pathsWindows = $state('');
	let pathsLinux = $state('');
	let pathsMacos = $state('');
	let excludes = $state('');
	let excludeFiles = $state('');
	let tags = $state('');
	let vss = $state(false);
	let oneFileSystem = $state(false);
	let excludeCaches = $state(false);
	let limitUpload = $state('0');
	let limitDownload = $state('0');
	let retentionEnabled = $state(false);
	let keepLast = $state('0');
	let keepDaily = $state('0');
	let keepWeekly = $state('0');
	let keepMonthly = $state('0');
	let keepWithin = $state('');
	let checkEnabled = $state(false);
	let readDataSubset = $state('');
	let hooks = $state<HookDraft[]>([]);

	let deleteTarget = $state<Profile | null>(null);
	let deleteOpen = $state(false);
	let deleting = $state(false);

	let previewOpen = $state(false);
	let previewTarget = $state<Profile | null>(null);
	let previewAgentId = $state('');
	let previewLoading = $state(false);
	let previewResult = $state<PreviewResponse | null>(null);
	let previewProblem = $state<ApiProblem | null>(null);

	const canWrite = $derived(['admin', 'operator'].includes(getSession()?.role ?? ''));
	const repositoryNames = $derived(new Map(repositories.map((item) => [item.id, item.name])));

	const repositoryOptions = $derived<SelectOption[]>(
		repositories.map((item) => ({ value: item.id, label: item.name }))
	);
	const phaseOptions = $derived<SelectOption[]>(
		PHASES.map((phase) => ({ value: phase, label: phaseLabel(phase) }))
	);
	const agentOptions = $derived<SelectOption[]>([
		{ value: '', label: t('profiles.previewHost') },
		...agents
			.filter((agent) => agent.status === 'active')
			.map((agent) => ({ value: agent.id, label: agent.hostname }))
	]);

	const columns = $derived<TableColumn<Profile>[]>([
		{ key: 'name', header: t('profiles.profile'), value: (row) => row.name, render: nameCell },
		{
			key: 'repository',
			header: t('profiles.repository'),
			value: (row) => repositoryNames.get(row.repository_id) ?? row.repository_id,
			render: repositoryCell
		},
		{ key: 'tags', header: t('profiles.tags'), value: (row) => row.tags.join(', ') },
		{
			key: 'retention',
			header: t('profiles.retention'),
			value: (row) => retentionSummary(row),
			render: retentionCell
		}
	]);

	const empty = $derived(!loading && !error && profiles.length === 0);

	function splitLines(value: string): string[] {
		return value
			.split('\n')
			.map((line) => line.trim())
			.filter((line) => line.length > 0);
	}

	function joinLines(values: string[]): string {
		return values.join('\n');
	}

	function phaseLabel(phase: HookPhase): string {
		switch (phase) {
			case 'before':
				return t('profiles.hooks.phase.before');
			case 'after':
				return t('profiles.hooks.phase.after');
			case 'on_success':
				return t('profiles.hooks.phase.on_success');
			case 'on_error':
				return t('profiles.hooks.phase.on_error');
			case 'finally':
				return t('profiles.hooks.phase.finally');
		}
	}

	function actionsFor(profile: Profile): MenuItem[] {
		const items: MenuItem[] = [
			{ label: t('profiles.preview'), icon: 'eye', onSelect: () => openPreview(profile) }
		];
		if (canWrite) {
			items.push({
				label: t('profiles.edit'),
				icon: 'settings',
				onSelect: () => openEdit(profile)
			});
			items.push({
				label: t('profiles.delete'),
				icon: 'octagon',
				tone: 'danger',
				onSelect: () => {
					deleteTarget = profile;
					deleteOpen = true;
				}
			});
		}
		return items;
	}

	function numberOrZero(value: string): number {
		const parsed = Number.parseInt(value, 10);
		return Number.isFinite(parsed) && parsed > 0 ? parsed : 0;
	}

	function retentionSummary(profile: Profile): string {
		if (!profile.retention) return t('common.none');
		const retention = profile.retention;
		const parts: string[] = [];
		if ((retention.keep_last ?? 0) > 0) parts.push(`last ${retention.keep_last}`);
		if ((retention.keep_daily ?? 0) > 0) parts.push(`daily ${retention.keep_daily}`);
		if ((retention.keep_weekly ?? 0) > 0) parts.push(`weekly ${retention.keep_weekly}`);
		if ((retention.keep_monthly ?? 0) > 0) parts.push(`monthly ${retention.keep_monthly}`);
		if (retention.keep_within) parts.push(`within ${retention.keep_within}`);
		return parts.length > 0 ? parts.join(', ') : t('common.none');
	}

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			const [profileList, repositoryList, agentList] = await Promise.all([
				service.listProfiles(),
				service.listRepositories(),
				service.listAgents()
			]);
			profiles = profileList;
			repositories = repositoryList;
			agents = agentList;
		} catch (problem) {
			error = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	onMount(load);

	$effect(() => {
		if (!formOpen) resetForm();
	});

	function resetForm(): void {
		name = '';
		repositoryId = '';
		pathsAll = '';
		pathsWindows = '';
		pathsLinux = '';
		pathsMacos = '';
		excludes = '';
		excludeFiles = '';
		tags = '';
		vss = false;
		oneFileSystem = false;
		excludeCaches = false;
		limitUpload = '0';
		limitDownload = '0';
		retentionEnabled = false;
		keepLast = '0';
		keepDaily = '0';
		keepWeekly = '0';
		keepMonthly = '0';
		keepWithin = '';
		checkEnabled = false;
		readDataSubset = '';
		hooks = [];
		formError = null;
		formProblem = null;
	}

	function openCreate(): void {
		editing = null;
		resetForm();
		repositoryId = repositories[0]?.id ?? '';
		formOpen = true;
	}

	function openEdit(profile: Profile): void {
		editing = profile;
		resetForm();
		name = profile.name;
		repositoryId = profile.repository_id;
		pathsAll = joinLines(profile.paths.all ?? []);
		pathsWindows = joinLines(profile.paths.windows ?? []);
		pathsLinux = joinLines(profile.paths.linux ?? []);
		pathsMacos = joinLines(profile.paths.macos ?? []);
		excludes = joinLines(profile.excludes ?? []);
		excludeFiles = joinLines(profile.exclude_files ?? []);
		tags = joinLines(profile.tags ?? []);
		vss = profile.options.use_fs_snapshot ?? false;
		oneFileSystem = profile.options.one_file_system ?? false;
		excludeCaches = profile.options.exclude_caches ?? false;
		limitUpload = String(profile.options.limit_upload_kib ?? 0);
		limitDownload = String(profile.options.limit_download_kib ?? 0);
		if (profile.retention) {
			retentionEnabled = true;
			keepLast = String(profile.retention.keep_last ?? 0);
			keepDaily = String(profile.retention.keep_daily ?? 0);
			keepWeekly = String(profile.retention.keep_weekly ?? 0);
			keepMonthly = String(profile.retention.keep_monthly ?? 0);
			keepWithin = profile.retention.keep_within ?? '';
		}
		if (profile.check) {
			checkEnabled = true;
			readDataSubset = profile.check.read_data_subset ?? '';
		}
		hooks = profile.hooks.map((hook) => ({
			hookId: hook.hook_id,
			phase: hook.phase,
			parameters: (hook.parameters ?? []).map((parameter) => ({ ...parameter }))
		}));
		formOpen = true;
	}

	function addHook(): void {
		hooks = [...hooks, { hookId: '', phase: 'before', parameters: [] }];
	}

	function removeHook(index: number): void {
		hooks = hooks.filter((_, position) => position !== index);
	}

	function addParameter(index: number): void {
		hooks = hooks.map((hook, position) =>
			position === index
				? { ...hook, parameters: [...hook.parameters, { name: '', value: '' }] }
				: hook
		);
	}

	function removeParameter(index: number, parameter: number): void {
		hooks = hooks.map((hook, position) =>
			position === index
				? { ...hook, parameters: hook.parameters.filter((_, item) => item !== parameter) }
				: hook
		);
	}

	function hookRefs(): HookRef[] {
		return hooks
			.filter((hook) => hook.hookId.trim().length > 0)
			.map((hook) => ({
				hook_id: hook.hookId.trim(),
				phase: hook.phase,
				parameters: hook.parameters
					.filter((parameter) => parameter.name.trim().length > 0)
					.map((parameter) => ({ name: parameter.name.trim(), value: parameter.value }))
			}));
	}

	function settings() {
		return {
			paths: {
				all: splitLines(pathsAll),
				windows: splitLines(pathsWindows),
				linux: splitLines(pathsLinux),
				macos: splitLines(pathsMacos)
			},
			excludes: splitLines(excludes),
			exclude_files: splitLines(excludeFiles),
			tags: splitLines(tags),
			options: {
				use_fs_snapshot: vss,
				one_file_system: oneFileSystem,
				exclude_caches: excludeCaches,
				limit_upload_kib: numberOrZero(limitUpload),
				limit_download_kib: numberOrZero(limitDownload)
			},
			hooks: hookRefs(),
			retention: retentionEnabled
				? {
						keep_last: numberOrZero(keepLast),
						keep_hourly: 0,
						keep_daily: numberOrZero(keepDaily),
						keep_weekly: numberOrZero(keepWeekly),
						keep_monthly: numberOrZero(keepMonthly),
						keep_yearly: 0,
						keep_within: keepWithin.trim(),
						keep_tags: [],
						group_by: [],
						filter: { hosts: [], tags: [], paths: [] }
					}
				: null,
			check: checkEnabled ? { read_data_subset: readDataSubset.trim() } : null
		};
	}

	async function save(): Promise<void> {
		saving = true;
		formError = null;
		formProblem = null;
		try {
			const shared = settings();
			if (editing) {
				const request: UpdateProfileRequest = {
					name,
					repository_id: repositoryId,
					...shared
				};
				await service.updateProfile(editing.id, request);
				pushToast('success', t('profiles.updated'));
			} else {
				const request: CreateProfileRequest = {
					name,
					repository_id: repositoryId,
					...shared
				};
				await service.createProfile(request);
				pushToast('success', t('profiles.created'));
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

	function openPreview(profile: Profile): void {
		previewTarget = profile;
		previewAgentId = '';
		previewResult = null;
		previewProblem = null;
		previewOpen = true;
	}

	async function runPreview(): Promise<void> {
		if (!previewTarget || !previewAgentId) return;
		previewLoading = true;
		previewProblem = null;
		previewResult = null;
		try {
			previewResult = await service.previewProfile(previewTarget.id, previewAgentId);
		} catch (problem) {
			previewProblem = problem instanceof ApiError ? problem.problem : null;
		} finally {
			previewLoading = false;
		}
	}
</script>

{#snippet nameCell(profile: Profile)}
	<span class="profile">
		<span class="profile__name">{profile.name}</span>
		<span class="profile__meta">{profile.tags.join(' · ')}</span>
	</span>
{/snippet}

{#snippet repositoryCell(profile: Profile)}
	<span>{repositoryNames.get(profile.repository_id) ?? profile.repository_id}</span>
{/snippet}

{#snippet retentionCell(profile: Profile)}
	<span class="profile__meta">{retentionSummary(profile)}</span>
{/snippet}

<svelte:head>
	<title>{t('profiles.title')} · {t('app.name')}</title>
</svelte:head>

<PageHeader
	title={t('profiles.title')}
	description={t('profiles.description')}
	count={loading ? undefined : profiles.length}
>
	{#if canWrite}
		<Button variant="primary" icon="plus" onclick={openCreate}>{t('profiles.add')}</Button>
	{/if}
</PageHeader>

{#if empty}
	<EmptyState
		icon="repository"
		title={t('profiles.emptyTitle')}
		description={t('profiles.emptyDescription')}
		bordered
	>
		{#if canWrite}
			<Button variant="primary" onclick={openCreate}>{t('profiles.addFirst')}</Button>
		{/if}
	</EmptyState>
{:else}
	<Table
		{columns}
		rows={profiles}
		{loading}
		{error}
		rowKey={(profile) => profile.id}
		filterPlaceholder={t('profiles.searchPlaceholder')}
	>
		{#snippet rowActions(profile)}
			<DropdownMenu
				icon="more"
				align="end"
				label={t('profiles.moreActions', { name: profile.name })}
				items={actionsFor(profile)}
			/>
		{/snippet}
	</Table>
{/if}

<Dialog
	bind:open={formOpen}
	size="lg"
	title={editing ? t('profiles.editTitle') : t('profiles.addTitle')}
	description={editing ? t('profiles.editDescription') : t('profiles.addDescription')}
	onsubmit={save}
>
	{#if formError}
		<Alert tone="danger" title={formError} />
	{/if}
	<div class="form">
		<TextField
			id="profile-name"
			label={t('profiles.profile')}
			error={fieldError(formProblem, 'name')}
			bind:value={name}
			required
		/>
		<Select
			id="profile-repository"
			label={t('profiles.repository')}
			options={repositoryOptions}
			error={fieldError(formProblem, 'repository_id')}
			bind:value={repositoryId}
			required
		/>

		<h3 class="form__section">{t('profiles.paths')}</h3>
		<Field
			label={t('profiles.pathsAll')}
			help={t('profiles.listSeparatorHint')}
			error={fieldError(formProblem, 'paths.all')}
			for="profile-paths-all"
		>
			<textarea id="profile-paths-all" class="list-input" bind:value={pathsAll}></textarea>
		</Field>
		<Field
			label={t('profiles.pathsWindows')}
			error={fieldError(formProblem, 'paths.windows')}
			for="profile-paths-windows"
		>
			<textarea id="profile-paths-windows" class="list-input" bind:value={pathsWindows}></textarea>
		</Field>
		<Field
			label={t('profiles.pathsLinux')}
			error={fieldError(formProblem, 'paths.linux')}
			for="profile-paths-linux"
		>
			<textarea id="profile-paths-linux" class="list-input" bind:value={pathsLinux}></textarea>
		</Field>
		<Field
			label={t('profiles.pathsMacos')}
			error={fieldError(formProblem, 'paths.macos')}
			for="profile-paths-macos"
		>
			<textarea id="profile-paths-macos" class="list-input" bind:value={pathsMacos}></textarea>
		</Field>
		<Field
			label={t('profiles.excludes')}
			help={t('profiles.listSeparatorHint')}
			error={fieldError(formProblem, 'excludes')}
			for="profile-excludes"
		>
			<textarea id="profile-excludes" class="list-input" bind:value={excludes}></textarea>
		</Field>
		<Field
			label={t('profiles.excludeFiles')}
			help={t('profiles.listSeparatorHint')}
			error={fieldError(formProblem, 'exclude_files')}
			for="profile-exclude-files"
		>
			<textarea id="profile-exclude-files" class="list-input" bind:value={excludeFiles}></textarea>
		</Field>
		<Field
			label={t('profiles.tags')}
			help={t('profiles.listSeparatorHint')}
			error={fieldError(formProblem, 'tags')}
			for="profile-tags"
		>
			<textarea id="profile-tags" class="list-input" bind:value={tags}></textarea>
		</Field>

		<h3 class="form__section">{t('profiles.options')}</h3>
		<Checkbox label={t('profiles.options.vss')} bind:checked={vss} />
		<Checkbox label={t('profiles.options.oneFileSystem')} bind:checked={oneFileSystem} />
		<Checkbox label={t('profiles.options.excludeCaches')} bind:checked={excludeCaches} />
		<TextField
			id="profile-limit-upload"
			type="number"
			label={t('profiles.options.limitUpload')}
			bind:value={limitUpload}
		/>
		<TextField
			id="profile-limit-download"
			type="number"
			label={t('profiles.options.limitDownload')}
			bind:value={limitDownload}
		/>

		<h3 class="form__section">{t('profiles.retention')}</h3>
		<Checkbox label={t('profiles.retention.enabled')} bind:checked={retentionEnabled} />
		{#if retentionEnabled}
			<div class="grid">
				<TextField
					id="profile-keep-last"
					type="number"
					label={t('profiles.retention.keepLast')}
					bind:value={keepLast}
				/>
				<TextField
					id="profile-keep-daily"
					type="number"
					label={t('profiles.retention.keepDaily')}
					bind:value={keepDaily}
				/>
				<TextField
					id="profile-keep-weekly"
					type="number"
					label={t('profiles.retention.keepWeekly')}
					bind:value={keepWeekly}
				/>
				<TextField
					id="profile-keep-monthly"
					type="number"
					label={t('profiles.retention.keepMonthly')}
					bind:value={keepMonthly}
				/>
			</div>
			<TextField
				id="profile-keep-within"
				label={t('profiles.retention.keepWithin')}
				placeholder="30d"
				error={fieldError(formProblem, 'retention.keep_within')}
				bind:value={keepWithin}
			/>
		{/if}

		<h3 class="form__section">{t('profiles.check')}</h3>
		<Checkbox label={t('profiles.check.enabled')} bind:checked={checkEnabled} />
		{#if checkEnabled}
			<TextField
				id="profile-read-data-subset"
				label={t('profiles.check.readDataSubset')}
				placeholder="10%"
				error={fieldError(formProblem, 'check.read_data_subset')}
				bind:value={readDataSubset}
			/>
		{/if}

		<h3 class="form__section">{t('profiles.hooks')}</h3>
		<Alert variant="inline" icon="lock" title={t('profiles.hooksUnavailable')} />
		{#each hooks as hook, index (index)}
			<div class="hook">
				<div class="hook__row">
					<TextField
						label={t('profiles.hooks.hookId')}
						error={fieldError(formProblem, 'hooks.hook_id')}
						bind:value={hook.hookId}
					/>
					<Select
						label={t('profiles.hooks.phase')}
						options={phaseOptions}
						bind:value={hook.phase}
					/>
					<Button variant="ghost" icon="octagon" onclick={() => removeHook(index)}>
						{t('profiles.hooks.remove')}
					</Button>
				</div>
				{#each hook.parameters as parameter, position (position)}
					<div class="hook__row">
						<TextField label={t('profiles.hooks.parameters')} bind:value={parameter.name} />
						<TextField label="" bind:value={parameter.value} />
						<Button variant="ghost" icon="x" onclick={() => removeParameter(index, position)}>
							{t('common.delete')}
						</Button>
					</div>
				{/each}
				<Button variant="ghost" icon="plus" onclick={() => addParameter(index)}>
					{t('profiles.hooks.parameters')}
				</Button>
			</div>
		{/each}
		<Button variant="ghost" icon="plus" onclick={addHook}>{t('profiles.hooks.add')}</Button>
	</div>
	{#snippet footer()}
		<Button onclick={() => (formOpen = false)}>{t('common.cancel')}</Button>
		<Button type="submit" variant="primary" loading={saving}>{t('common.save')}</Button>
	{/snippet}
</Dialog>

<SidePanel
	bind:open={previewOpen}
	title={t('profiles.previewTitle')}
	subtitle={t('profiles.previewDescription')}
>
	<div class="preview">
		<Select
			id="preview-host"
			label={t('profiles.previewHost')}
			options={agentOptions}
			bind:value={previewAgentId}
			onchange={runPreview}
		/>
		{#if previewProblem}
			<Alert tone="danger" title={tErrorCode(previewProblem.code) ?? previewProblem.title}>
				{previewProblem.field ? `${previewProblem.field}: ${previewProblem.reason ?? ''}` : ''}
			</Alert>
		{/if}
		{#if previewLoading}
			<p class="preview__hint">{t('common.loading')}</p>
		{:else if previewResult}
			{#each previewResult.steps as step (step.kind + step.command)}
				<div class="step">
					<div class="step__head">
						<span class="step__kind">{step.kind}</span>
						{#if step.maintenance}
							<Badge tone="info">{t('profiles.previewMaintenance')}</Badge>
						{/if}
					</div>
					<div class="step__label">{t('profiles.previewCommand')}</div>
					<pre class="step__command">{step.command}</pre>
					{#if step.environment.length > 0}
						<div class="step__label">{t('profiles.previewEnvironment')}</div>
						<ul class="step__env">
							{#each step.environment as entry (entry.name)}
								<li><span class="step__env-name">{entry.name}</span> = {entry.value}</li>
							{/each}
						</ul>
					{/if}
				</div>
			{/each}
		{:else}
			<p class="preview__hint">{t('profiles.previewEmpty')}</p>
		{/if}
	</div>
</SidePanel>

<ConfirmDialog
	bind:open={deleteOpen}
	title={t('profiles.deleteTitle')}
	message={t('profiles.deleteBody', { name: deleteTarget?.name ?? '' })}
	confirmLabel={t('profiles.delete')}
	tone="danger"
	loading={deleting}
	onConfirm={confirmDelete}
/>

<style>
	.profile {
		display: flex;
		flex-direction: column;
	}

	.profile__name {
		font-weight: var(--rh-font-weight-semibold);
		white-space: nowrap;
	}

	.profile__meta {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.form {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-4);
	}

	.form__section {
		margin: var(--rh-space-2) 0 0;
		font-size: var(--rh-font-size-sm);
		font-weight: var(--rh-font-weight-semibold);
		color: var(--rh-color-text-muted);
	}

	.grid {
		display: grid;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		gap: var(--rh-space-3);
	}

	.list-input {
		width: 100%;
		min-height: 4.5rem;
		padding: var(--rh-space-2);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-control-bg);
		color: var(--rh-color-text);
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-sm);
		resize: vertical;
	}

	.hook {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-2);
		padding: var(--rh-space-3);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
	}

	.hook__row {
		display: flex;
		align-items: flex-end;
		gap: var(--rh-space-2);
	}

	.preview {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-4);
	}

	.preview__hint {
		color: var(--rh-color-text-muted);
	}

	.step {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-2);
		padding: var(--rh-space-3);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
	}

	.step__head {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.step__kind {
		font-weight: var(--rh-font-weight-semibold);
	}

	.step__label {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.step__command {
		margin: 0;
		padding: var(--rh-space-2);
		overflow-x: auto;
		border-radius: var(--rh-radius-sm);
		background-color: var(--rh-color-surface-sunken, var(--rh-color-control-bg));
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-sm);
		white-space: pre-wrap;
		word-break: break-all;
	}

	.step__env {
		margin: 0;
		padding-left: var(--rh-space-4);
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-sm);
	}

	.step__env-name {
		color: var(--rh-color-text-muted);
	}
</style>
