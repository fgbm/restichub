<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import { nameError } from '#lib/names.ts';
	import type {
		Agent,
		ApiProblem,
		CreateProfileRequest,
		JobLimitDefaults,
		JobTemplate,
		Plan,
		PreviewRequest,
		PreviewResponse,
		Profile,
		ProfileWarning,
		Repository,
		ScriptTemplate,
		Secret,
		UpdateProfileRequest
	} from '#lib/api/index.ts';
	import HostPicker from './HostPicker.svelte';
	import ProfileSection from './ProfileSection.svelte';
	import RepositoryPicker from './RepositoryPicker.svelte';
	import RetentionEditor from './RetentionEditor.svelte';
	import RichText from './RichText.svelte';
	import ScriptsEditor from './ScriptsEditor.svelte';
	import VariableTags from './VariableTags.svelte';
	import VariableTextarea from './VariableTextarea.svelte';
	import { RecordPage, SummaryPanel } from './layout/index.ts';
	import {
		Alert,
		Badge,
		Button,
		Checkbox,
		ConfirmDialog,
		DataTable,
		Disclosure,
		Icon,
		Select,
		SidePanel,
		TextField
	} from './ui/index.ts';
	import type { KeyValueItem, SelectOption, TableColumn } from './ui/index.ts';
	import { formatMinutes } from '#lib/format.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import {
		SUMMARY_ORDER,
		blankParameter,
		declaredParameters,
		draftFromProfile,
		emptyDraft,
		enabledCount,
		parameterUses,
		settingsOf,
		splitLines,
		summarize,
		minutesToHours,
		MAX_TAGS
	} from '#lib/profile-form.ts';
	import type { ProfileDraft, ProfileParameterDraft, SummaryKey } from '#lib/profile-form.ts';
	import {
		isParameterName,
		overridingPlans,
		parameterNameError,
		plansRefusal,
		undeclaredNames,
		usageText
	} from '#lib/parameters.ts';
	import { hostLabels, secretReference } from '#lib/variables.ts';
	import { rememberSections } from '#lib/section-state.svelte.ts';
	import { loadPreference, savePreference } from '#lib/stores/ui-preferences.svelte.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';

	let {
		profileId = null,
		copyFrom = null
	}: {
		profileId?: string | null;
		/** A new profile starts as a copy of this profile. */
		copyFrom?: string | null;
	} = $props();

	const SECTIONS = SUMMARY_ORDER;

	let loading = $state(true);
	let loadError = $state<string | null>(null);
	let existing = $state<Profile | null>(null);
	let repositories = $state<Repository[]>([]);
	let agents = $state<Agent[]>([]);
	let plans = $state<Plan[]>([]);
	let scriptTemplates = $state<ScriptTemplate[]>([]);
	let jobTemplates = $state<JobTemplate[]>([]);
	let secrets = $state<Secret[]>([]);

	let limitDefaults = $state<JobLimitDefaults | undefined>();
	let draft = $state<ProfileDraft>(emptyDraft());
	let baseline = $state('');
	let saving = $state(false);
	let formError = $state<string | null>(null);
	let formProblem = $state<ApiProblem | null>(null);
	let warnings = $state<ProfileWarning[]>([]);
	let copied = $state(false);
	const open = rememberSections(
		'profile',
		SECTIONS,
		Object.fromEntries(SECTIONS.map((key) => [key, key === 'basics' || key === 'paths']))
	);

	let excludeListOpen = $state(false);

	const canWrite = $derived(['admin', 'operator'].includes(getSession()?.role ?? ''));
	// Code that runs on a host needs the Admin role (ADR-0037).
	const canEditScripts = $derived(['admin', 'platform_admin'].includes(getSession()?.role ?? ''));
	const planCount = $derived(
		existing ? plans.filter((plan) => plan.profile_id === existing?.id).length : 0
	);
	const scriptTargets = $derived.by(() => {
		if (!existing) return [];
		const ids = new Set(
			plans.filter((plan) => plan.profile_id === existing?.id).map((plan) => plan.agent_id)
		);
		return agents.filter((agent) => ids.has(agent.id));
	});
	const scriptCounts = $derived({
		before: draft.scripts.filter((script) => script.phase === 'before').length,
		after: draft.scripts.filter((script) => script.phase !== 'before').length
	});
	const summaries = $derived(summarize(draft, scriptCounts, limitDefaults));
	const counts = $derived(enabledCount(summaries));
	const activeHosts = $derived(agents.filter((agent) => agent.status === 'active'));
	const commandRepository = $derived(repositories.find((item) => item.id === commandRepositoryId));
	const stateKey = $derived(
		JSON.stringify([draft.name, settingsOf(draft, { canEditScripts: true, existing })])
	);
	const dirty = $derived(!loading && stateKey !== baseline);
	// A copy is a record that is not saved yet, even before the first change.
	const unsavedCopy = $derived(Boolean(copyFrom) && !existing && !loading);
	const allOpen = $derived(SECTIONS.every((key) => open[key]));

	const titles: Record<SummaryKey, MessageKey> = {
		basics: 'pe.section.basics',
		paths: 'pe.section.paths',
		excludes: 'pe.section.excludes',
		tags: 'profiles.tags',
		options: 'pe.section.options',
		retention: 'profiles.retention',
		check: 'pe.section.check',
		scripts: 'pe.section.scripts',
		parameters: 'pe.section.parameters',
		limits: 'pe.section.limits'
	};

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
		try {
			const [profileList, repositoryList, agentList, planList, defaults] = await Promise.all([
				service.listProfiles(),
				service.listRepositories(),
				service.listAgents(),
				service.listPlans(),
				// Without the defaults the fields stay empty and the server decides.
				service.getJobLimitDefaults().catch(() => undefined)
			]);
			limitDefaults = defaults;
			repositories = repositoryList;
			agents = agentList;
			plans = planList;
			// The commands panel opens with the host that the person kept, else the host of the first
			// plan of the profile, else the first online host (AC-UI-11).
			const active = agentList.filter((agent) => agent.status === 'active');
			if (!active.some((agent) => agent.id === commandHostId)) {
				const planned = profileId
					? planList.find((plan) => plan.profile_id === profileId)?.agent_id
					: undefined;
				commandHostId =
					active.find((agent) => agent.id === planned)?.id ??
					active.find((agent) => agent.online)?.id ??
					active[0]?.id ??
					'';
			}
			commandRepositoryId =
				repositoryList.find((item) => item.initialized)?.id ?? repositoryList[0]?.id ?? '';
			if (profileId) {
				const found = profileList.find((item) => item.id === profileId);
				if (!found) {
					loadError = t('pe.notFound');
					return;
				}
				existing = found;
				draft = draftFromProfile(found);
			} else {
				const source = copyFrom ? profileList.find((item) => item.id === copyFrom) : undefined;
				if (source) {
					draft = draftFromProfile(source);
					draft.name = t('profiles.copyName', { name: source.name });
				} else {
					draft = emptyDraft(defaults);
				}
			}
			// The templates and the secrets serve the script editor only. A failure leaves the
			// editor without choices and does not block the page.
			const [templateList, jobTemplateList, secretList] = await Promise.allSettled([
				service.listScriptTemplates(),
				service.listJobTemplates(),
				service.listSecrets()
			]);
			scriptTemplates = templateList.status === 'fulfilled' ? templateList.value : [];
			jobTemplates = jobTemplateList.status === 'fulfilled' ? jobTemplateList.value : [];
			secrets = secretList.status === 'fulfilled' ? secretList.value : [];
			baseline = JSON.stringify([
				draft.name,
				settingsOf(draft, { canEditScripts: true, existing })
			]);
		} catch (problem) {
			loadError = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	// A script that the operator adds to a switched-off section turns the section on.
	let scriptCount = 0;
	$effect(() => {
		const count = draft.scripts.length;
		if (count > scriptCount && !draft.enabled.scripts) draft.enabled.scripts = true;
		scriptCount = count;
	});

	onMount(load);

	// The list file stays folded until it holds a value or an error.
	$effect(() => {
		if (draft.excludeFiles !== '' || formProblem?.field === 'exclude_files') excludeListOpen = true;
	});

	// A section that holds a server error opens, so the message is not hidden.
	$effect(() => {
		const field = formProblem?.field ?? '';
		const target: [string, string][] = [
			['name', 'basics'],
			['paths', 'paths'],
			['excludes', 'excludes'],
			['exclude_files', 'excludes'],
			['tags', 'tags'],
			['options.limit', 'options'],
			['options.stall', 'limits'],
			['options.max', 'limits'],
			['retention', 'retention'],
			['check', 'check'],
			['scripts', 'scripts'],
			['parameters', 'parameters']
		];
		const hit = target.find(([prefix]) => field.startsWith(prefix));
		if (hit) open[hit[1]] = true;
	});

	function setAll(value: boolean): void {
		for (const key of SECTIONS) open[key] = value;
	}

	// The parameters of the draft: one row of the table each, with the server error of its fields.
	const kindOptions = $derived<SelectOption[]>([
		{ value: 'text', label: t('parameters.kind.text') },
		{ value: 'secret', label: t('parameters.kind.secret') }
	]);
	const uses = $derived(parameterUses(draft));
	const declaredNames = $derived(
		draft.parameters.map((parameter) => parameter.name.trim()).filter((name) => name !== '')
	);
	const listParameters = $derived(declaredParameters(draft));
	// The keys of the label variables come from the hosts of the plans of the profile. A copy or a
	// new profile has no plans, so the list offers the typed key only.
	const planHosts = $derived.by(() => {
		if (!existing) return [];
		const ids = new Set(
			plans.filter((plan) => plan.profile_id === existing?.id).map((plan) => plan.agent_id)
		);
		return agents.filter((agent) => ids.has(agent.id));
	});
	const hostLabelKeys = $derived(hostLabels(planHosts));
	const secretNames = $derived(
		draft.parameters
			.filter((parameter) => parameter.kind === 'secret')
			.map((parameter) => parameter.name.trim())
			.filter((name) => name !== '')
	);

	// A reference that the draft does not declare gets a hint under its field, and the person
	// declares it there; nothing is declared automatically (FR-PF-10).
	function undeclaredIn(text: string): string[] {
		return undeclaredNames(splitLines(text), declaredNames);
	}

	// A secret never goes into a path or a tag, because restic shows its arguments (FR-AG-06). The
	// server refuses the save; the field says it as soon as the reference is complete (AC-PF-09).
	function secretReferenceIn(text: string): string | undefined {
		const name = secretReference(text, secretNames);
		return name ? t('pe.vars.secretError', { name }) : undefined;
	}

	function declareParameter(name: string): void {
		if (declaredNames.includes(name)) return;
		const row = blankParameter();
		row.name = name;
		row.required = true;
		draft.parameters = [...draft.parameters, row];
		open.parameters = true;
		document
			.getElementById('sec-parameters')
			?.scrollIntoView?.({ behavior: 'smooth', block: 'start' });
	}

	function parameterIndexOf(row: ProfileParameterDraft): number {
		return draft.parameters.findIndex((parameter) => parameter.key === row.key);
	}

	function parameterError(row: ProfileParameterDraft, field: string): string | undefined {
		return fieldError(formProblem, `parameters[${parameterIndexOf(row)}].${field}`);
	}

	function addParameter(): void {
		draft.parameters = [...draft.parameters, blankParameter()];
		open.parameters = true;
	}

	function removeParameter(row: ProfileParameterDraft): void {
		draft.parameters = draft.parameters.filter((parameter) => parameter.key !== row.key);
	}

	// Only a text parameter holds a default value (ADR-0047), so a switch to a secret clears it.
	function changeParameterKind(row: ProfileParameterDraft, kind: string): void {
		row.kind = kind === 'secret' ? 'secret' : 'text';
		if (row.kind === 'secret') row.defaultValue = '';
	}

	// A removed or renamed parameter drops the values that the plans of the profile hold for it
	// (FR-PF-10), so the person confirms which plans lose them.
	const lostOverrides = $derived.by(() => {
		if (!existing) return null;
		const removed = (existing.parameters ?? [])
			.map((parameter) => parameter.name)
			.filter((name) => !declaredNames.includes(name));
		if (removed.length === 0) return null;
		const found = overridingPlans(plans, existing.id, removed);
		if (found.links.length === 0 && found.more === 0) return null;
		return { names: removed, plans: found };
	});
	let confirmOverrides = $state(false);

	function requestSave(): void {
		if (lostOverrides) {
			confirmOverrides = true;
			return;
		}
		void save();
	}

	async function save(): Promise<void> {
		saving = true;
		formError = null;
		formProblem = null;
		try {
			const shared = settingsOf(draft, { canEditScripts, existing });
			const request = { name: draft.name, ...shared };
			const response = existing
				? await service.updateProfile(existing.id, request as UpdateProfileRequest)
				: await service.createProfile(request as CreateProfileRequest);
			pushToast('success', t(existing ? 'profiles.updated' : 'profiles.created'));
			if (response.warnings.length > 0) {
				// The warnings stay on the page. The operator reads them before leaving.
				warnings = response.warnings;
				existing = response.profile;
				baseline = stateKey;
				return;
			}
			await goto(resolve('/(app)/profiles'));
		} catch (problem) {
			formProblem = problemOf(problem);
			formError = formProblem ? null : messageOf(problem);
		} finally {
			saving = false;
		}
	}

	// The server refused the save because plans would stay without a required parameter (FR-PF-11).
	const refused = $derived(plansRefusal(formProblem));

	const resultItems = $derived<KeyValueItem[]>(
		SUMMARY_ORDER.map((key) => ({ key, label: t(titles[key]), value: summaries[key].text }))
	);

	// The server compiles the draft for one host, so the panel shows the same command that the agent
	// runs, with the secrets masked (FR-PF-04, AC-PF-01).
	let commandsOpen = $state(false);
	let previewLoading = $state(false);
	let previewResult = $state<PreviewResponse | null>(null);
	let previewProblem = $state<ApiProblem | null>(null);
	let previewSequence = 0;
	// AC-UI-11 keeps the host of the commands panel for the browser.
	let commandHostId = $state(
		loadPreference(
			'browser',
			'commands.host',
			(value): value is string => typeof value === 'string',
			''
		)
	);
	let commandRepositoryId = $state('');

	function selectCommandHost(id: string): void {
		commandHostId = id;
		savePreference('browser', 'commands.host', id);
	}

	const previewMessage = $derived.by(() => {
		const problem = previewProblem;
		if (!problem) return null;
		const reason = problem.field ? fieldError(problem, problem.field) : undefined;
		return reason ? `${problem.field} · ${reason}` : (tErrorCode(problem.code) ?? problem.title);
	});
	// The whole draft is the input of a writer, so a change of any setting refreshes the commands.
	// A role without the write right changes no setting: the server previews the stored profile.
	const previewKey = $derived(
		JSON.stringify([
			commandHostId,
			commandRepositoryId,
			canWrite ? draft.name : null,
			canWrite ? settingsOf(draft, { canEditScripts, existing }) : null
		])
	);
	let lastPreviewKey = '';

	async function runPreview(): Promise<void> {
		if (!commandHostId) {
			previewResult = null;
			return;
		}
		// The panel and the debounce both ask for the commands, so one key is sent one time.
		if (previewKey === lastPreviewKey) return;
		lastPreviewKey = previewKey;
		const sequence = ++previewSequence;
		const request: PreviewRequest = {
			agent_id: commandHostId,
			profile_id: existing?.id,
			repository_id: commandRepositoryId || undefined,
			...(canWrite ? { name: draft.name, ...settingsOf(draft, { canEditScripts, existing }) } : {})
		};
		previewLoading = true;
		previewProblem = null;
		try {
			const result = await service.previewProfile(request);
			if (sequence === previewSequence) previewResult = result;
		} catch (problem) {
			if (sequence !== previewSequence) return;
			previewResult = null;
			previewProblem = problemOf(problem);
		} finally {
			if (sequence === previewSequence) previewLoading = false;
		}
	}

	// A person types a path or a tag one key at a time, so the server runs after a pause.
	$effect(() => {
		void previewKey;
		if (!commandsOpen) return;
		const timer = setTimeout(() => void runPreview(), 400);
		return () => clearTimeout(timer);
	});

	const hasPaths = $derived(
		[draft.pathsAll, draft.pathsWindows, draft.pathsLinux, draft.pathsMacos].some(
			(text) => text.trim() !== ''
		)
	);

	async function copyCommands(): Promise<void> {
		const commands = previewResult?.steps.map((step) => step.command) ?? [];
		await navigator.clipboard?.writeText(commands.join('\n\n'));
		copied = true;
		setTimeout(() => (copied = false), 1200);
	}
</script>

{#snippet parameterHint(names: string[])}
	{#each names as name (name)}
		<p class="hint">
			<span>{t('pe.param.notDeclared', { name })}</span>
			<!-- The server refuses a name that its rule does not allow, so the button shows only
			     for a valid one (FR-PF-09). The list under the field applies the same rule. -->
			{#if isParameterName(name)}
				<Button size="sm" variant="ghost" onclick={() => declareParameter(name)}>
					{t('pe.param.declare')}
				</Button>
			{/if}
		</p>
	{/each}
{/snippet}

{#snippet parameterNameCell(row: ProfileParameterDraft)}
	<TextField
		value={row.name}
		oninput={(event) => (row.name = event.currentTarget.value)}
		mono
		aria-label={t('parameters.name')}
		error={parameterNameError(row.name) ?? parameterError(row, 'name')}
		placeholder={t('pe.param.nameExample')}
	/>
{/snippet}

{#snippet parameterKindCell(row: ProfileParameterDraft)}
	<span class="param-kind">
		<Select
			value={row.kind}
			options={kindOptions}
			onchange={(event) => changeParameterKind(row, event.currentTarget.value)}
			aria-label={t('parameters.kind')}
		/>
	</span>
{/snippet}

{#snippet parameterRequiredCell(row: ProfileParameterDraft)}
	<Checkbox bind:checked={row.required} aria-label={t('parameters.required')} />
{/snippet}

{#snippet parameterDefaultCell(row: ProfileParameterDraft)}
	{#if row.kind === 'secret'}
		<TextField
			disabled
			value={t('parameters.defaultLocked')}
			aria-label={t('parameters.default')}
			error={parameterError(row, 'default_value')}
		/>
	{:else}
		<TextField
			value={row.defaultValue}
			oninput={(event) => (row.defaultValue = event.currentTarget.value)}
			mono
			aria-label={t('parameters.default')}
			placeholder={t('pe.param.defaultExample')}
			error={parameterError(row, 'default_value')}
		/>
	{/if}
{/snippet}

{#snippet parameterUsedCell(row: ProfileParameterDraft)}
	<span class="muted nowrap"
		>{usageText(
			uses[row.name.trim()] ?? { paths: 0, excludes: 0, excludeFiles: 0, tags: 0, scripts: 0 }
		)}</span
	>
{/snippet}

{#snippet parameterDescriptionNote(row: ProfileParameterDraft)}
	<TextField
		value={row.description}
		oninput={(event) => (row.description = event.currentTarget.value)}
		aria-label={t('parameters.description')}
		placeholder={t('pe.param.descriptionExample')}
		error={parameterError(row, 'description')}
	/>
{/snippet}

<RecordPage
	title={profileId ? (existing?.name ?? '') : t('pe.newTitle')}
	description={t('profiles.description')}
	{loading}
	error={loadError}
	onsubmit={requestSave}
	save={{
		dirty: dirty || unsavedCopy,
		saving,
		create: !profileId,
		readonly: canWrite ? null : t('pe.save.readonly'),
		note: existing && planCount > 0 ? t('pe.save.plans', { count: planCount }) : undefined,
		oncancel: () => goto(resolve('/(app)/profiles'))
	}}
>
	{#snippet actions()}
		<Button size="sm" onclick={() => setAll(true)} disabled={allOpen}>{t('pe.expandAll')}</Button>
		<Button size="sm" onclick={() => setAll(false)}>{t('pe.collapseAll')}</Button>
		<Button
			size="sm"
			icon="terminal"
			onclick={() => {
				commandsOpen = true;
				void runPreview();
			}}
			disabled={!hasPaths}
			title={hasPaths ? undefined : t('pe.commands.noPaths')}
		>
			{t('pe.side.commands')}
		</Button>
	{/snippet}
	{#snippet banners()}
		{#if formError}
			<Alert tone="danger" title={formError} />
		{/if}
		{#if refused}
			<Alert tone="danger" title={t('pe.param.refusedTitle', { names: refused.names.join(', ') })}>
				<ul class="blockers">
					{#each refused.plans.links as link (link.href + link.label)}
						<li>
							<span class="blockers__kind">{link.kind}</span>
							<!-- The refusal resolves the address of the plan with `resolve()`. -->
							<!-- eslint-disable-next-line svelte/no-navigation-without-resolve -->
							<a href={link.href}>{link.label}</a>
						</li>
					{/each}
					{#if refused.plans.more > 0}
						<li class="blockers__more">{t('deleteBlocked.more', { n: refused.plans.more })}</li>
					{/if}
				</ul>
				<p class="blockers__hint">{t('pe.param.refusedHint')}</p>
			</Alert>
		{/if}
		{#if warnings.length > 0}
			<Alert tone="attention" title={t('profiles.warningsTitle')}>
				<ul class="warnings">
					{#each warnings as warning (warning.field + warning.reason + (warning.os ?? ''))}
						<li>{warning.field} · {warning.reason}{warning.os ? ` · ${warning.os}` : ''}</li>
					{/each}
				</ul>
			</Alert>
		{/if}
	{/snippet}
	{#snippet side()}
		<SummaryPanel title={t('pe.side.result')} items={resultItems}>
			<p class="muted">{t('pe.side.count', { on: counts.on, total: counts.total })}</p>
		</SummaryPanel>
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
			id="profile-name"
			label={t('profiles.profile')}
			help={t('names.help')}
			error={nameError(draft.name) ?? fieldError(formProblem, 'name')}
			bind:value={draft.name}
			required
		/>
	</ProfileSection>

	<ProfileSection
		id="paths"
		title={t(titles.paths)}
		summary={summaries.paths.text}
		state={summaries.paths.state}
		required
		bind:open={open.paths}
	>
		<p class="lead">{t('pe.paths.lead')}</p>
		<VariableTextarea
			id="profile-paths-all"
			label={t('profiles.pathsAll')}
			help={t('pe.vars.help')}
			error={fieldError(formProblem, 'paths.all') ?? secretReferenceIn(draft.pathsAll)}
			rows={2}
			parameters={listParameters}
			labels={hostLabelKeys}
			ondeclare={declareParameter}
			bind:value={draft.pathsAll}
		/>
		{@render parameterHint(undeclaredIn(draft.pathsAll))}
		<div class="grid3">
			<VariableTextarea
				id="profile-paths-windows"
				label={t('profiles.pathsWindows')}
				error={fieldError(formProblem, 'paths.windows') ?? secretReferenceIn(draft.pathsWindows)}
				rows={3}
				parameters={listParameters}
				labels={hostLabelKeys}
				ondeclare={declareParameter}
				bind:value={draft.pathsWindows}
			/>
			{@render parameterHint(undeclaredIn(draft.pathsWindows))}
			<VariableTextarea
				id="profile-paths-linux"
				label={t('profiles.pathsLinux')}
				error={fieldError(formProblem, 'paths.linux') ?? secretReferenceIn(draft.pathsLinux)}
				rows={3}
				parameters={listParameters}
				labels={hostLabelKeys}
				ondeclare={declareParameter}
				bind:value={draft.pathsLinux}
			/>
			{@render parameterHint(undeclaredIn(draft.pathsLinux))}
			<VariableTextarea
				id="profile-paths-macos"
				label={t('profiles.pathsMacos')}
				error={fieldError(formProblem, 'paths.macos') ?? secretReferenceIn(draft.pathsMacos)}
				rows={3}
				parameters={listParameters}
				labels={hostLabelKeys}
				ondeclare={declareParameter}
				bind:value={draft.pathsMacos}
			/>
			{@render parameterHint(undeclaredIn(draft.pathsMacos))}
		</div>
	</ProfileSection>

	<ProfileSection
		id="excludes"
		title={t(titles.excludes)}
		summary={summaries.excludes.text}
		state={summaries.excludes.state}
		switchable
		bind:enabled={draft.enabled.excludes}
		bind:open={open.excludes}
	>
		<p class="lead">{t('pe.excludes.lead')}</p>
		<VariableTextarea
			id="profile-excludes"
			label={t('profiles.excludes')}
			help={t('pe.excludes.patternsHelp')}
			error={fieldError(formProblem, 'excludes') ?? secretReferenceIn(draft.excludes)}
			rows={4}
			parameters={listParameters}
			labels={hostLabelKeys}
			ondeclare={declareParameter}
			bind:value={draft.excludes}
		/>
		{@render parameterHint(undeclaredIn(draft.excludes))}
		<Disclosure label={t('pe.excludes.more')} bind:open={excludeListOpen}>
			<VariableTextarea
				id="profile-exclude-files"
				label={t('profiles.excludeFiles')}
				help={t('pe.excludes.filesHelp')}
				error={fieldError(formProblem, 'exclude_files') ?? secretReferenceIn(draft.excludeFiles)}
				rows={3}
				parameters={listParameters}
				labels={hostLabelKeys}
				ondeclare={declareParameter}
				bind:value={draft.excludeFiles}
			/>
			{@render parameterHint(undeclaredIn(draft.excludeFiles))}
		</Disclosure>
	</ProfileSection>

	<ProfileSection
		id="tags"
		title={t(titles.tags)}
		summary={summaries.tags.text}
		state={summaries.tags.state}
		switchable
		bind:enabled={draft.enabled.tags}
		bind:open={open.tags}
	>
		<p class="lead">{t('pe.tags.lead')}</p>
		<VariableTags
			id="profile-tags"
			label={t('profiles.tags')}
			help={t('pe.tags.help')}
			placeholder={t('pe.tags.placeholder')}
			max={MAX_TAGS}
			error={fieldError(formProblem, 'tags') ?? secretReferenceIn(draft.tags.join('\n'))}
			parameters={listParameters}
			labels={hostLabelKeys}
			ondeclare={declareParameter}
			bind:value={draft.tags}
		/>
		{@render parameterHint(undeclaredIn(draft.tags.join('\n')))}
	</ProfileSection>

	<ProfileSection
		id="options"
		title={t(titles.options)}
		summary={summaries.options.text}
		state={summaries.options.state}
		switchable
		bind:enabled={draft.enabled.options}
		bind:open={open.options}
	>
		<div class="checks">
			<Checkbox
				label={t('profiles.options.vss')}
				help={t('pe.options.vssHelp')}
				bind:checked={draft.vss}
			/>
			<Checkbox
				label={t('profiles.options.oneFileSystem')}
				help={t('pe.options.oneFsHelp')}
				bind:checked={draft.oneFileSystem}
			/>
			<Checkbox
				label={t('profiles.options.excludeCaches')}
				help={t('pe.options.cachesHelp')}
				bind:checked={draft.excludeCaches}
			/>
			<Checkbox
				label={t('profiles.options.lowPriority')}
				help={t('pe.options.lowPriorityHelp')}
				bind:checked={draft.lowPriority}
			/>
		</div>
		<div class="grid2">
			<TextField
				id="profile-limit-upload"
				type="number"
				min="0"
				label={t('profiles.options.limitUpload')}
				placeholder={t('profiles.options.limitNone')}
				error={fieldError(formProblem, 'options.limit_upload_kib')}
				bind:value={draft.limitUpload}
			/>
			<TextField
				id="profile-limit-download"
				type="number"
				min="0"
				label={t('profiles.options.limitDownload')}
				placeholder={t('profiles.options.limitNone')}
				error={fieldError(formProblem, 'options.limit_download_kib')}
				bind:value={draft.limitDownload}
			/>
		</div>
	</ProfileSection>

	<ProfileSection
		id="retention"
		title={t(titles.retention)}
		summary={summaries.retention.text}
		state={summaries.retention.state}
		switchable
		bind:enabled={draft.enabled.retention}
		bind:open={open.retention}
	>
		<RetentionEditor bind:draft problem={formProblem} />
	</ProfileSection>

	<ProfileSection
		id="check"
		title={t(titles.check)}
		summary={summaries.check.text}
		state={summaries.check.state}
		switchable
		bind:enabled={draft.enabled.check}
		bind:open={open.check}
	>
		<p class="lead">{t('pe.check.lead')}</p>
		<TextField
			id="profile-read-data-subset"
			mono
			label={t('profiles.check.readDataSubset')}
			help={t('pe.check.help')}
			placeholder={t('pe.check.placeholder')}
			error={fieldError(formProblem, 'check.read_data_subset')}
			bind:value={draft.readDataSubset}
		/>
	</ProfileSection>

	<ProfileSection
		id="scripts"
		title={t(titles.scripts)}
		summary={summaries.scripts.text}
		state={summaries.scripts.state}
		switchable
		bind:enabled={draft.enabled.scripts}
		bind:open={open.scripts}
	>
		{#if formProblem?.field?.startsWith('scripts')}
			<Alert tone="danger" title={tErrorCode(formProblem.code) ?? formProblem.title}>
				{formProblem.field}{formProblem.reason ? `: ${formProblem.reason}` : ''}
			</Alert>
		{/if}
		<ScriptsEditor
			bind:scripts={draft.scripts}
			bind:secrets
			templates={scriptTemplates}
			{jobTemplates}
			targets={scriptTargets}
			parameters={declaredParameters(draft)}
			readonly={!canEditScripts}
			problem={formProblem}
			framed={false}
		/>
	</ProfileSection>

	<ProfileSection
		id="parameters"
		title={t(titles.parameters)}
		summary={summaries.parameters.text}
		state={summaries.parameters.state}
		badge={draft.parameters.length > 0 ? String(draft.parameters.length) : undefined}
		bind:open={open.parameters}
	>
		<p class="lead">{t('pe.param.hint')}</p>
		{#if draft.parameters.length > 0}
			<DataTable
				paginated={false}
				rowKey={(row) => String(row.key)}
				rows={draft.parameters}
				columns={[
					{ key: 'name', header: t('parameters.name'), sortable: false, render: parameterNameCell },
					{
						key: 'kind',
						header: t('parameters.kind'),
						sortable: false,
						nowrap: true,
						render: parameterKindCell
					},
					{
						key: 'required',
						header: t('parameters.required'),
						sortable: false,
						align: 'center',
						render: parameterRequiredCell
					},
					{
						key: 'default',
						header: t('parameters.default'),
						sortable: false,
						render: parameterDefaultCell
					},
					{
						key: 'used',
						header: t('parameters.used'),
						sortable: false,
						nowrap: true,
						render: parameterUsedCell
					}
				] satisfies TableColumn<ProfileParameterDraft>[]}
				rowNote={parameterDescriptionNote}
				actions={(row) => [
					{
						id: 'remove',
						label: t('parameters.remove', { name: row.name.trim() }),
						hint: t('parameters.remove', { name: row.name.trim() }),
						icon: 'trash',
						danger: true,
						onSelect: () => removeParameter(row)
					}
				]}
			/>
		{:else}
			<p class="muted">{t('pe.param.summaryNone')}</p>
		{/if}
		<Alert variant="inline" tone="info" title={t('pe.param.visibility')} />
		<div>
			<Button variant="ghost" icon="plus" onclick={addParameter}>{t('pe.param.add')}</Button>
		</div>
		<p class="muted">{t('pe.param.nameRule')}</p>
	</ProfileSection>

	<ProfileSection
		id="limits"
		title={t(titles.limits)}
		summary={summaries.limits.text}
		state={summaries.limits.state}
		bind:open={open.limits}
	>
		<p class="lead">{t('pe.limits.lead')}</p>
		<div class="grid2">
			<TextField
				id="profile-stall-timeout"
				type="number"
				label={t('pe.limits.stall')}
				help={limitDefaults
					? t('pe.limits.stallHelp', {
							value: formatMinutes(limitDefaults.stall_timeout_minutes)
						})
					: undefined}
				placeholder={limitDefaults
					? t('pe.limits.stallPlaceholder', {
							count: limitDefaults.stall_timeout_minutes
						})
					: undefined}
				error={fieldError(formProblem, 'options.stall_timeout_minutes')}
				bind:value={draft.stallMinutes}
			/>
			<TextField
				id="profile-max-duration"
				type="number"
				step="any"
				label={t('pe.limits.max')}
				help={limitDefaults
					? t('pe.limits.maxHelp', { value: formatMinutes(limitDefaults.max_duration_minutes) })
					: undefined}
				placeholder={limitDefaults
					? t('pe.limits.maxPlaceholder', {
							count: minutesToHours(limitDefaults.max_duration_minutes)
						})
					: undefined}
				error={fieldError(formProblem, 'options.max_duration_minutes')}
				bind:value={draft.maxDurationHours}
			/>
		</div>
		<p class="muted"><RichText text={t('pe.limits.env')} /></p>
	</ProfileSection>

	<SidePanel
		bind:open={commandsOpen}
		title={t('pe.side.commands')}
		subtitle={t('pe.commands.description')}
	>
		<div class="commands">
			<div class="grid2">
				<HostPicker
					id="commands-host"
					hosts={activeHosts}
					value={commandHostId}
					onchange={selectCommandHost}
				/>
				<RepositoryPicker
					id="commands-repository"
					{repositories}
					{agents}
					bind:value={commandRepositoryId}
				/>
			</div>
			{#if !commandRepository}
				<p class="muted">{t('pe.commands.noRepository')}</p>
			{/if}
			{#if previewMessage}
				<Alert tone="danger" title={previewMessage} />
			{:else if previewLoading}
				<p class="muted">{t('common.loading')}</p>
			{:else if previewResult}
				{#each previewResult.steps as step, index (step.kind + index)}
					<div class="step">
						<div class="step__head">
							<span class="step__kind">{step.kind}</span>
							{#if step.maintenance}
								<Badge tone="info">{t('profiles.previewMaintenance')}</Badge>
							{/if}
						</div>
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
				<p class="muted">{t('profiles.previewEmpty')}</p>
			{/if}
			{#if !existing && previewResult}
				<p class="muted note">{t('pe.commands.unsavedTag')}</p>
			{/if}
			<p class="muted note">
				<Icon name="info" size="xs" />
				{t('pe.commands.visibility')}
			</p>
			<p class="muted note">
				<Icon name="clock" size="xs" />
				{summaries.limits.text}
			</p>
		</div>
		{#snippet footer()}
			<Button icon="copy" onclick={copyCommands}>
				{copied ? t('pe.side.copied') : t('pe.side.copy')}
			</Button>
		{/snippet}
	</SidePanel>
</RecordPage>

<ConfirmDialog
	bind:open={confirmOverrides}
	title={t('pe.param.removeTitle')}
	message={t('pe.param.removeBody', { names: (lostOverrides?.names ?? []).join(', ') })}
	confirmLabel={t('pe.param.removeConfirm')}
	links={lostOverrides?.plans.links ?? []}
	more={lostOverrides?.plans.more ?? 0}
	onConfirm={() => {
		confirmOverrides = false;
		void save();
	}}
/>

<style>
	.note {
		margin: 0;
	}

	.muted {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.lead {
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	/* A table cell cuts the select to its narrowest width, which hides the end of the longest
	   option, "Секрет" or "Secret". The cell keeps the width of that option instead. */
	.param-kind {
		display: block;
		min-width: max-content;
	}

	.grid2 {
		display: grid;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		gap: var(--rh-space-3);
	}

	.grid3 {
		display: grid;
		grid-template-columns: repeat(3, minmax(0, 1fr));
		gap: var(--rh-space-3);
	}

	.checks {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
	}

	.blockers {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-1);
		margin: var(--rh-space-2) 0 0;
		padding: 0;
		list-style: none;
	}

	.blockers__kind {
		margin-inline-end: var(--rh-space-2);
		color: var(--rh-color-text-muted);
	}

	.blockers__more,
	.blockers__hint {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.blockers__hint {
		margin: var(--rh-space-2) 0 0;
	}

	.warnings {
		margin: 0;
		padding-left: var(--rh-space-4);
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-sm);
	}

	@media (max-width: 720px) {
		.grid2,
		.grid3 {
			grid-template-columns: minmax(0, 1fr);
		}
	}

	.hint {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--rh-space-2);
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.commands {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-4);
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
