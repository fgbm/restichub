<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import type {
		Agent,
		ApiProblem,
		CreateProfileRequest,
		GroupBy,
		HookPhase,
		HookRef,
		JobTemplate,
		Plan,
		PreviewResponse,
		Profile,
		ProfileWarning,
		ScriptTemplate,
		Secret,
		UpdateProfileRequest
	} from '#lib/api/index.ts';
	import ScriptsEditor from '#lib/components/ScriptsEditor.svelte';
	import { draftOf, scriptOf } from '#lib/scripts.ts';
	import type { ScriptDraft } from '#lib/scripts.ts';
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

	type HookDraft = { hookId: string; phase: HookPhase };

	const PHASES: HookPhase[] = ['before', 'after', 'on_success', 'on_error', 'finally'];

	let profiles = $state<Profile[]>([]);
	let repositories = $state<{ id: string; name: string }[]>([]);
	let agents = $state<Agent[]>([]);
	let plans = $state<Plan[]>([]);
	let scriptTemplates = $state<ScriptTemplate[]>([]);
	let jobTemplates = $state<JobTemplate[]>([]);
	let secrets = $state<Secret[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let savedWarnings = $state<ProfileWarning[]>([]);

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
	let limitUpload = $state('');
	let limitDownload = $state('');
	let stallMinutes = $state('60');
	let maxDurationMinutes = $state('');
	let retentionEnabled = $state(false);
	let keepLast = $state('0');
	let keepHourly = $state('0');
	let keepDaily = $state('0');
	let keepWeekly = $state('0');
	let keepMonthly = $state('0');
	let keepYearly = $state('0');
	let keepWithin = $state('');
	let keepTags = $state('');
	let groupByHost = $state(false);
	let groupByPaths = $state(false);
	let groupByTags = $state(false);
	let filterHosts = $state('');
	let filterTags = $state('');
	let filterPaths = $state('');
	let checkEnabled = $state(false);
	let readDataSubset = $state('');
	let hooks = $state<HookDraft[]>([]);
	let scripts = $state<ScriptDraft[]>([]);

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
	// Code that runs on a host needs the Admin role (ADR-0037).
	const canEditScripts = $derived(['admin', 'platform_admin'].includes(getSession()?.role ?? ''));
	// The hosts that run the profile in the form: the hosts of its plans.
	const scriptTargets = $derived.by(() => {
		if (!editing) return [];
		const ids = new Set(
			plans.filter((plan) => plan.profile_id === editing?.id).map((plan) => plan.agent_id)
		);
		return agents.filter((agent) => ids.has(agent.id));
	});
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

	function joinLines(values: string[] | undefined): string {
		return (values ?? []).join('\n');
	}

	function numberOrZero(value: string): number {
		const parsed = Number.parseInt(value, 10);
		return Number.isFinite(parsed) && parsed > 0 ? parsed : 0;
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

	function retentionSummary(profile: Profile): string {
		if (!profile.retention) return t('common.none');
		const retention = profile.retention;
		const parts: string[] = [];
		if ((retention.keep_last ?? 0) > 0)
			parts.push(t('profiles.retention.summary.last', { count: retention.keep_last ?? 0 }));
		if ((retention.keep_daily ?? 0) > 0)
			parts.push(t('profiles.retention.summary.daily', { count: retention.keep_daily ?? 0 }));
		if ((retention.keep_weekly ?? 0) > 0)
			parts.push(t('profiles.retention.summary.weekly', { count: retention.keep_weekly ?? 0 }));
		if ((retention.keep_monthly ?? 0) > 0)
			parts.push(t('profiles.retention.summary.monthly', { count: retention.keep_monthly ?? 0 }));
		if ((retention.keep_yearly ?? 0) > 0)
			parts.push(t('profiles.retention.summary.yearly', { count: retention.keep_yearly ?? 0 }));
		if (retention.keep_within)
			parts.push(t('profiles.retention.summary.within', { value: retention.keep_within }));
		return parts.length > 0 ? parts.join(', ') : t('common.none');
	}

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	function problemOf(problem: unknown): ApiProblem | null {
		if (problem instanceof ApiError) return problem.problem;
		if (problem && typeof problem === 'object' && 'problem' in problem) {
			return (problem as { problem: ApiProblem }).problem;
		}
		return null;
	}

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			const [profileList, repositoryList, agentList, planList] = await Promise.all([
				service.listProfiles(),
				service.listRepositories(),
				service.listAgents(),
				service.listPlans()
			]);
			profiles = profileList;
			repositories = repositoryList;
			agents = agentList;
			plans = planList;
			// The templates and the secrets serve the script editor only. A failure leaves the
			// editor without choices and does not block the list.
			const [templateList, jobTemplateList, secretList] = await Promise.allSettled([
				service.listScriptTemplates(),
				service.listJobTemplates(),
				service.listSecrets()
			]);
			scriptTemplates = templateList.status === 'fulfilled' ? templateList.value : [];
			jobTemplates = jobTemplateList.status === 'fulfilled' ? jobTemplateList.value : [];
			secrets = secretList.status === 'fulfilled' ? secretList.value : [];
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
		limitUpload = '';
		limitDownload = '';
		stallMinutes = '60';
		maxDurationMinutes = '';
		retentionEnabled = false;
		keepLast = '0';
		keepHourly = '0';
		keepDaily = '0';
		keepWeekly = '0';
		keepMonthly = '0';
		keepYearly = '0';
		keepWithin = '';
		keepTags = '';
		groupByHost = false;
		groupByPaths = false;
		groupByTags = false;
		filterHosts = '';
		filterTags = '';
		filterPaths = '';
		checkEnabled = false;
		readDataSubset = '';
		hooks = [];
		scripts = [];
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
		pathsAll = joinLines(profile.paths?.all);
		pathsWindows = joinLines(profile.paths?.windows);
		pathsLinux = joinLines(profile.paths?.linux);
		pathsMacos = joinLines(profile.paths?.macos);
		excludes = joinLines(profile.excludes);
		excludeFiles = joinLines(profile.exclude_files);
		tags = joinLines(profile.tags);
		vss = profile.options?.use_fs_snapshot ?? false;
		oneFileSystem = profile.options?.one_file_system ?? false;
		excludeCaches = profile.options?.exclude_caches ?? false;
		limitUpload = profile.options?.limit_upload_kib ? String(profile.options.limit_upload_kib) : '';
		limitDownload = profile.options?.limit_download_kib
			? String(profile.options.limit_download_kib)
			: '';
		stallMinutes = String(profile.options?.stall_timeout_minutes ?? 60);
		maxDurationMinutes = profile.options?.max_duration_minutes
			? String(profile.options.max_duration_minutes)
			: '';
		if (profile.retention) {
			const retention = profile.retention;
			retentionEnabled = true;
			keepLast = String(retention.keep_last ?? 0);
			keepHourly = String(retention.keep_hourly ?? 0);
			keepDaily = String(retention.keep_daily ?? 0);
			keepWeekly = String(retention.keep_weekly ?? 0);
			keepMonthly = String(retention.keep_monthly ?? 0);
			keepYearly = String(retention.keep_yearly ?? 0);
			keepWithin = retention.keep_within ?? '';
			keepTags = joinLines(retention.keep_tags);
			groupByHost = (retention.group_by ?? []).includes('host');
			groupByPaths = (retention.group_by ?? []).includes('paths');
			groupByTags = (retention.group_by ?? []).includes('tags');
			filterHosts = joinLines(retention.filter?.hosts);
			filterTags = joinLines(retention.filter?.tags);
			filterPaths = joinLines(retention.filter?.paths);
		}
		if (profile.check) {
			checkEnabled = true;
			readDataSubset = profile.check.read_data_subset ?? '';
		}
		hooks = (profile.hooks ?? []).map((hook) => ({ hookId: hook.hook_id, phase: hook.phase }));
		scripts = (profile.scripts ?? []).map((script) => draftOf(script));
		formOpen = true;
	}

	function addHook(): void {
		hooks = [...hooks, { hookId: '', phase: 'before' }];
	}

	function removeHook(index: number): void {
		hooks = hooks.filter((_, position) => position !== index);
	}

	function groupBy(): GroupBy[] {
		const keys: GroupBy[] = [];
		if (groupByHost) keys.push('host');
		if (groupByPaths) keys.push('paths');
		if (groupByTags) keys.push('tags');
		return keys;
	}

	function hookRefs(): HookRef[] {
		return hooks
			.filter((hook) => hook.hookId.trim().length > 0)
			.map((hook) => ({ hook_id: hook.hookId.trim(), phase: hook.phase, parameters: [] }));
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
				limit_download_kib: numberOrZero(limitDownload),
				stall_timeout_minutes: numberOrZero(stallMinutes) || 60,
				max_duration_minutes: numberOrZero(maxDurationMinutes)
			},
			hooks: hookRefs(),
			// An Operator keeps the scripts as they are stored: only an Admin changes them.
			scripts: canEditScripts ? scripts.map(scriptOf) : (editing?.scripts ?? []),
			retention: retentionEnabled
				? {
						keep_last: numberOrZero(keepLast),
						keep_hourly: numberOrZero(keepHourly),
						keep_daily: numberOrZero(keepDaily),
						keep_weekly: numberOrZero(keepWeekly),
						keep_monthly: numberOrZero(keepMonthly),
						keep_yearly: numberOrZero(keepYearly),
						keep_within: keepWithin.trim(),
						keep_tags: splitLines(keepTags),
						group_by: groupBy(),
						filter: {
							hosts: splitLines(filterHosts),
							tags: splitLines(filterTags),
							paths: splitLines(filterPaths)
						}
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
			const response = editing
				? await service.updateProfile(editing.id, {
						name,
						repository_id: repositoryId,
						...shared
					} satisfies UpdateProfileRequest)
				: await service.createProfile({
						name,
						repository_id: repositoryId,
						...shared
					} satisfies CreateProfileRequest);
			savedWarnings = response.warnings;
			pushToast('success', t(editing ? 'profiles.updated' : 'profiles.created'));
			formOpen = false;
			resetForm();
			await load();
		} catch (problem) {
			formProblem = problemOf(problem);
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
			previewProblem = problemOf(problem);
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

{#if savedWarnings.length > 0}
	<div class="warnings">
		<Alert tone="attention" title={t('profiles.warningsTitle')}>
			<ul class="warnings__list">
				{#each savedWarnings as warning (warning.field + warning.reason + (warning.os ?? ''))}
					<li>
						{warning.field} · {warning.reason}{warning.os ? ` · ${warning.os}` : ''}
					</li>
				{/each}
			</ul>
		</Alert>
	</div>
{/if}

{#if empty}
	<EmptyState
		icon="checklist"
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
	size="xl"
	title={editing ? t('profiles.editTitle') : t('profiles.addTitle')}
	description={editing ? t('profiles.editDescription') : t('profiles.addDescription')}
	onsubmit={save}
>
	{#if formError}
		<Alert tone="danger" title={formError} />
	{/if}
	<div class="form">
		<section class="section">
			<h3 class="section__title">{t('profiles.section.source')}</h3>
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
			<Field
				label={t('profiles.pathsAll')}
				help={t('profiles.listSeparatorHint')}
				error={fieldError(formProblem, 'paths.all')}
				for="profile-paths-all"
			>
				<textarea id="profile-paths-all" class="rh-textarea rh-input--mono" bind:value={pathsAll}
				></textarea>
			</Field>
			<div class="grid">
				<Field
					label={t('profiles.pathsWindows')}
					error={fieldError(formProblem, 'paths.windows')}
					for="profile-paths-windows"
				>
					<textarea
						id="profile-paths-windows"
						class="rh-textarea rh-input--mono"
						bind:value={pathsWindows}></textarea>
				</Field>
				<Field
					label={t('profiles.pathsLinux')}
					error={fieldError(formProblem, 'paths.linux')}
					for="profile-paths-linux"
				>
					<textarea
						id="profile-paths-linux"
						class="rh-textarea rh-input--mono"
						bind:value={pathsLinux}></textarea>
				</Field>
				<Field
					label={t('profiles.pathsMacos')}
					error={fieldError(formProblem, 'paths.macos')}
					for="profile-paths-macos"
				>
					<textarea
						id="profile-paths-macos"
						class="rh-textarea rh-input--mono"
						bind:value={pathsMacos}></textarea>
				</Field>
			</div>
		</section>

		<section class="section">
			<h3 class="section__title">{t('profiles.section.exclusions')}</h3>
			<Field
				label={t('profiles.excludes')}
				help={t('profiles.listSeparatorHint')}
				error={fieldError(formProblem, 'excludes')}
				for="profile-excludes"
			>
				<textarea id="profile-excludes" class="rh-textarea rh-input--mono" bind:value={excludes}
				></textarea>
			</Field>
			<Field
				label={t('profiles.excludeFiles')}
				help={t('profiles.listSeparatorHint')}
				error={fieldError(formProblem, 'exclude_files')}
				for="profile-exclude-files"
			>
				<textarea
					id="profile-exclude-files"
					class="rh-textarea rh-input--mono"
					bind:value={excludeFiles}></textarea>
			</Field>
			<Field
				label={t('profiles.tags')}
				help={t('profiles.listSeparatorHint')}
				error={fieldError(formProblem, 'tags')}
				for="profile-tags"
			>
				<textarea id="profile-tags" class="rh-textarea rh-input--mono" bind:value={tags}></textarea>
			</Field>
		</section>

		<section class="section">
			<h3 class="section__title">{t('profiles.options')}</h3>
			<Checkbox label={t('profiles.options.vss')} bind:checked={vss} />
			<Checkbox label={t('profiles.options.oneFileSystem')} bind:checked={oneFileSystem} />
			<Checkbox label={t('profiles.options.excludeCaches')} bind:checked={excludeCaches} />
			<div class="grid">
				<TextField
					id="profile-limit-upload"
					type="number"
					label={t('profiles.options.limitUpload')}
					placeholder={t('profiles.options.limitNone')}
					bind:value={limitUpload}
				/>
				<TextField
					id="profile-limit-download"
					type="number"
					label={t('profiles.options.limitDownload')}
					placeholder={t('profiles.options.limitNone')}
					bind:value={limitDownload}
				/>
			</div>
		</section>

		<section class="section">
			<h3 class="section__title">{t('profiles.limits')}</h3>
			<p class="hint">{t('profiles.limits.hint')}</p>
			<div class="grid">
				<TextField
					id="profile-stall-timeout"
					type="number"
					min="5"
					label={t('profiles.limits.stall')}
					help={t('profiles.limits.stallHelp')}
					error={fieldError(formProblem, 'options.stall_timeout_minutes')}
					bind:value={stallMinutes}
				/>
				<TextField
					id="profile-max-duration"
					type="number"
					min="0"
					label={t('profiles.limits.maxDuration')}
					help={t('profiles.limits.maxDurationHelp')}
					placeholder={t('profiles.limits.none')}
					error={fieldError(formProblem, 'options.max_duration_minutes')}
					bind:value={maxDurationMinutes}
				/>
			</div>
		</section>

		<section class="section">
			<h3 class="section__title">{t('profiles.retention')}</h3>
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
						id="profile-keep-hourly"
						type="number"
						label={t('profiles.retention.keepHourly')}
						bind:value={keepHourly}
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
					<TextField
						id="profile-keep-yearly"
						type="number"
						label={t('profiles.retention.keepYearly')}
						bind:value={keepYearly}
					/>
				</div>
				<TextField
					id="profile-keep-within"
					label={t('profiles.retention.keepWithin')}
					placeholder="30d"
					error={fieldError(formProblem, 'retention.keep_within')}
					bind:value={keepWithin}
				/>
				<Field
					label={t('profiles.retention.keepTags')}
					help={t('profiles.listSeparatorHint')}
					error={fieldError(formProblem, 'retention.keep_tags')}
					for="profile-keep-tags"
				>
					<textarea id="profile-keep-tags" class="rh-textarea rh-input--mono" bind:value={keepTags}
					></textarea>
				</Field>
				<div class="checks">
					<span class="checks__label">{t('profiles.retention.groupBy')}</span>
					<Checkbox label={t('profiles.retention.groupBy.host')} bind:checked={groupByHost} />
					<Checkbox label={t('profiles.retention.groupBy.paths')} bind:checked={groupByPaths} />
					<Checkbox label={t('profiles.retention.groupBy.tags')} bind:checked={groupByTags} />
				</div>
				<Field
					label={t('profiles.retention.filterHosts')}
					help={t('profiles.listSeparatorHint')}
					error={fieldError(formProblem, 'retention.filter.hosts')}
					for="profile-filter-hosts"
				>
					<textarea
						id="profile-filter-hosts"
						class="rh-textarea rh-input--mono"
						bind:value={filterHosts}></textarea>
				</Field>
				<Field
					label={t('profiles.retention.filterTags')}
					help={t('profiles.listSeparatorHint')}
					error={fieldError(formProblem, 'retention.filter.tags')}
					for="profile-filter-tags"
				>
					<textarea
						id="profile-filter-tags"
						class="rh-textarea rh-input--mono"
						bind:value={filterTags}></textarea>
				</Field>
				<Field
					label={t('profiles.retention.filterPaths')}
					help={t('profiles.listSeparatorHint')}
					error={fieldError(formProblem, 'retention.filter.paths')}
					for="profile-filter-paths"
				>
					<textarea
						id="profile-filter-paths"
						class="rh-textarea rh-input--mono"
						bind:value={filterPaths}></textarea>
				</Field>
			{/if}
		</section>

		<section class="section">
			<h3 class="section__title">{t('profiles.check')}</h3>
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
		</section>

		<section class="section">
			<h3 class="section__title">{t('scripts.title')}</h3>
			{#if formProblem?.field?.startsWith('scripts')}
				<Alert tone="danger" title={tErrorCode(formProblem.code) ?? formProblem.title}>
					{formProblem.field}{formProblem.reason ? `: ${formProblem.reason}` : ''}
				</Alert>
			{/if}
			<ScriptsEditor
				bind:scripts
				bind:secrets
				templates={scriptTemplates}
				{jobTemplates}
				targets={scriptTargets}
				readonly={!canEditScripts}
				problem={formProblem}
			/>
		</section>

		<section class="section">
			<h3 class="section__title">{t('profiles.hooks')}</h3>
			<Alert variant="inline" icon="lock" title={t('profiles.hooksUnavailable')} />
			{#if formProblem?.field?.startsWith('hooks')}
				<Alert tone="danger" title={tErrorCode(formProblem.code) ?? formProblem.title}>
					{formProblem.field}{formProblem.reason ? `: ${formProblem.reason}` : ''}
				</Alert>
			{/if}
			{#each hooks as hook, index (index)}
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
			{/each}
			<Button variant="ghost" icon="plus" onclick={addHook}>{t('profiles.hooks.add')}</Button>
		</section>
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
				{previewProblem.field}{previewProblem.reason ? `: ${previewProblem.reason}` : ''}
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

	.warnings {
		margin-bottom: var(--rh-space-4);
	}

	.warnings__list {
		margin: 0;
		padding-left: var(--rh-space-4);
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-sm);
	}

	.form {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-5);
	}

	.section {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
	}

	.hint {
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
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

	.grid {
		display: grid;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		gap: var(--rh-space-3);
	}

	.checks {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--rh-space-3);
	}

	.checks__label {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
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
