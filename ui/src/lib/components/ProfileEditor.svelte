<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import type {
		Agent,
		ApiProblem,
		CreateProfileRequest,
		JobTemplate,
		Plan,
		Profile,
		ProfileWarning,
		ScriptTemplate,
		Secret,
		UpdateProfileRequest
	} from '#lib/api/index.ts';
	import ProfileSection from './ProfileSection.svelte';
	import RetentionEditor from './RetentionEditor.svelte';
	import RichText from './RichText.svelte';
	import ScriptsEditor from './ScriptsEditor.svelte';
	import TagInput from './TagInput.svelte';
	import {
		Alert,
		Button,
		Checkbox,
		Field,
		Icon,
		PageHeader,
		Select,
		Skeleton,
		TextField
	} from './ui/index.ts';
	import type { SelectOption } from './ui/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import {
		MAX_DURATION_DEFAULT_HOURS,
		STALL_DEFAULT_MINUTES,
		SUMMARY_ORDER,
		buildSteps,
		draftFromProfile,
		emptyDraft,
		enabledCount,
		settingsOf,
		summarize
	} from '#lib/profile-form.ts';
	import type { Os, ProfileDraft, SummaryKey } from '#lib/profile-form.ts';
	import { setPageTitle } from '#lib/stores/page-title.svelte.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';

	let { profileId = null }: { profileId?: string | null } = $props();

	const SECTIONS = SUMMARY_ORDER;

	let loading = $state(true);
	let loadError = $state<string | null>(null);
	let existing = $state<Profile | null>(null);
	let repositories = $state<{ id: string; name: string }[]>([]);
	let agents = $state<Agent[]>([]);
	let plans = $state<Plan[]>([]);
	let scriptTemplates = $state<ScriptTemplate[]>([]);
	let jobTemplates = $state<JobTemplate[]>([]);
	let secrets = $state<Secret[]>([]);

	let draft = $state<ProfileDraft>(emptyDraft());
	let baseline = $state('');
	let saving = $state(false);
	let formError = $state<string | null>(null);
	let formProblem = $state<ApiProblem | null>(null);
	let warnings = $state<ProfileWarning[]>([]);
	let os = $state('linux');
	let copied = $state(false);
	let open = $state<Record<string, boolean>>(
		Object.fromEntries(SECTIONS.map((key) => [key, key === 'basics' || key === 'paths']))
	);

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
	const repositoryOptions = $derived<SelectOption[]>(
		repositories.map((item) => ({ value: item.id, label: item.name }))
	);
	const repositoryName = $derived(
		repositories.find((item) => item.id === draft.repositoryId)?.name ?? ''
	);
	const scriptCounts = $derived({
		before: draft.scripts.filter((script) => script.phase === 'before').length,
		after: draft.scripts.filter((script) => script.phase !== 'before').length
	});
	const summaries = $derived(summarize(draft, repositoryName, scriptCounts));
	const counts = $derived(enabledCount(summaries));
	const steps = $derived(
		buildSteps(
			draft,
			os as Os,
			draft.scripts.map((script) => ({
				name: script.name,
				phase: script.phase === 'before' ? ('before' as const) : ('after' as const)
			}))
		)
	);
	const stateKey = $derived(
		JSON.stringify([
			draft.name,
			draft.repositoryId,
			settingsOf(draft, { canEditScripts: true, existing })
		])
	);
	const dirty = $derived(!loading && stateKey !== baseline);
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
			const [profileList, repositoryList, agentList, planList] = await Promise.all([
				service.listProfiles(),
				service.listRepositories(),
				service.listAgents(),
				service.listPlans()
			]);
			repositories = repositoryList;
			agents = agentList;
			plans = planList;
			if (profileId) {
				const found = profileList.find((item) => item.id === profileId);
				if (!found) {
					loadError = t('pe.notFound');
					return;
				}
				existing = found;
				draft = draftFromProfile(found);
			} else {
				draft = emptyDraft(repositoryList[0]?.id ?? '');
			}
			setPageTitle(profileId ? draft.name : t('pe.newTitle'));
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
				draft.repositoryId,
				settingsOf(draft, { canEditScripts: true, existing })
			]);
		} catch (problem) {
			loadError = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	onMount(load);
	onDestroy(() => setPageTitle(null));

	// A section that holds a server error opens, so the message is not hidden.
	$effect(() => {
		const field = formProblem?.field ?? '';
		const target: [string, string][] = [
			['name', 'basics'],
			['repository_id', 'basics'],
			['paths', 'paths'],
			['excludes', 'excludes'],
			['exclude_files', 'excludes'],
			['tags', 'tags'],
			['options.limit', 'options'],
			['options.stall', 'limits'],
			['options.max', 'limits'],
			['retention', 'retention'],
			['check', 'check'],
			['scripts', 'scripts']
		];
		const hit = target.find(([prefix]) => field.startsWith(prefix));
		if (hit) open[hit[1]] = true;
	});

	function setAll(value: boolean): void {
		for (const key of SECTIONS) open[key] = value;
	}

	async function save(): Promise<void> {
		saving = true;
		formError = null;
		formProblem = null;
		try {
			const shared = settingsOf(draft, { canEditScripts, existing });
			const request = {
				name: draft.name,
				repository_id: draft.repositoryId,
				...shared
			};
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

	async function copyCommands(): Promise<void> {
		await navigator.clipboard?.writeText(steps.map((step) => step.command).join('\n\n'));
		copied = true;
		setTimeout(() => (copied = false), 1200);
	}
</script>

<svelte:head>
	<title>{(profileId ? draft.name : '') || t('pe.newTitle')} · {t('app.name')}</title>
</svelte:head>

<PageHeader
	title={profileId ? t('pe.editTitle') : t('pe.newTitle')}
	description={t('pe.description')}
>
	<Button size="sm" onclick={() => setAll(true)} disabled={allOpen}>{t('pe.expandAll')}</Button>
	<Button size="sm" onclick={() => setAll(false)}>{t('pe.collapseAll')}</Button>
</PageHeader>

{#if loading}
	<Skeleton />
{:else if loadError}
	<Alert tone="danger" title={loadError} />
	<p><a href={resolve('/(app)/profiles')}>{t('pe.backToList')}</a></p>
{:else}
	{#if formError}
		<Alert tone="danger" title={formError} />
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
	<form
		class="layout"
		onsubmit={(event) => {
			event.preventDefault();
			void save();
		}}
	>
		<div class="main">
			<ProfileSection
				id="basics"
				title={t(titles.basics)}
				summary={summaries.basics.text}
				state={summaries.basics.state}
				required
				bind:open={open.basics}
			>
				<div class="grid2">
					<TextField
						id="profile-name"
						label={t('profiles.profile')}
						help={t('pe.name.help')}
						error={fieldError(formProblem, 'name')}
						bind:value={draft.name}
						required
					/>
					<Select
						id="profile-repository"
						label={t('profiles.repository')}
						options={repositoryOptions}
						error={fieldError(formProblem, 'repository_id')}
						bind:value={draft.repositoryId}
						required
					/>
				</div>
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
				<Field
					label={t('profiles.pathsAll')}
					error={fieldError(formProblem, 'paths.all')}
					for="profile-paths-all"
				>
					<textarea
						id="profile-paths-all"
						class="rh-textarea rh-input--mono"
						rows="2"
						bind:value={draft.pathsAll}></textarea>
				</Field>
				<div class="grid3">
					<Field
						label={t('profiles.pathsWindows')}
						error={fieldError(formProblem, 'paths.windows')}
						for="profile-paths-windows"
					>
						<textarea
							id="profile-paths-windows"
							class="rh-textarea rh-input--mono"
							rows="3"
							bind:value={draft.pathsWindows}></textarea>
					</Field>
					<Field
						label={t('profiles.pathsLinux')}
						error={fieldError(formProblem, 'paths.linux')}
						for="profile-paths-linux"
					>
						<textarea
							id="profile-paths-linux"
							class="rh-textarea rh-input--mono"
							rows="3"
							bind:value={draft.pathsLinux}></textarea>
					</Field>
					<Field
						label={t('profiles.pathsMacos')}
						error={fieldError(formProblem, 'paths.macos')}
						for="profile-paths-macos"
					>
						<textarea
							id="profile-paths-macos"
							class="rh-textarea rh-input--mono"
							rows="3"
							bind:value={draft.pathsMacos}></textarea>
					</Field>
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
				<p class="lead"><RichText text={t('pe.excludes.lead')} /></p>
				<div class="grid2">
					<Field
						label={t('profiles.excludes')}
						error={fieldError(formProblem, 'excludes')}
						for="profile-excludes"
					>
						<textarea
							id="profile-excludes"
							class="rh-textarea rh-input--mono"
							rows="3"
							bind:value={draft.excludes}></textarea>
					</Field>
					<Field
						label={t('profiles.excludeFiles')}
						help={t('pe.excludes.filesHelp')}
						error={fieldError(formProblem, 'exclude_files')}
						for="profile-exclude-files"
					>
						<textarea
							id="profile-exclude-files"
							class="rh-textarea rh-input--mono"
							rows="3"
							bind:value={draft.excludeFiles}></textarea>
					</Field>
				</div>
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
				<TagInput
					id="profile-tags"
					label={t('profiles.tags')}
					help={t('pe.tags.help')}
					error={fieldError(formProblem, 'tags')}
					bind:value={draft.tags}
				/>
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
					readonly={!canEditScripts}
					problem={formProblem}
					framed={false}
				/>
			</ProfileSection>

			<ProfileSection
				id="limits"
				title={t(titles.limits)}
				summary={summaries.limits.text}
				state={summaries.limits.state}
				badge={t('pe.new')}
				bind:open={open.limits}
			>
				<p class="lead">{t('pe.limits.lead')}</p>
				<div class="grid2">
					<TextField
						id="profile-stall-timeout"
						type="number"
						min="5"
						label={t('pe.limits.stall')}
						help={t('pe.limits.stallHelp', { count: STALL_DEFAULT_MINUTES })}
						placeholder={t('pe.limits.stallPlaceholder', { count: STALL_DEFAULT_MINUTES })}
						error={fieldError(formProblem, 'options.stall_timeout_minutes')}
						bind:value={draft.stallMinutes}
					/>
					<TextField
						id="profile-max-duration"
						type="number"
						min="1"
						label={t('pe.limits.max')}
						help={t('pe.limits.maxHelp', { count: MAX_DURATION_DEFAULT_HOURS })}
						placeholder={t('pe.limits.maxPlaceholder', { count: MAX_DURATION_DEFAULT_HOURS })}
						error={fieldError(formProblem, 'options.max_duration_minutes')}
						bind:value={draft.maxDurationHours}
					/>
				</div>
			</ProfileSection>
		</div>

		<aside class="side">
			<div class="box">
				<div class="box__head">
					<span>{t('pe.side.result')}</span>
					<span class="muted">{t('pe.side.count', { on: counts.on, total: counts.total })}</span>
				</div>
				<ul class="result">
					{#each SUMMARY_ORDER as key (key)}
						<li>
							<span class="dot dot--{summaries[key].state}" data-state={summaries[key].state}
							></span>
							<div>
								<b>{t(titles[key])}</b>
								<span class="result__text">{summaries[key].text}</span>
							</div>
						</li>
					{/each}
				</ul>
			</div>
			<div class="box">
				<div class="box__head">
					<span>{t('pe.side.commands')}</span>
					<Button size="sm" icon="copy" onclick={copyCommands}>
						{copied ? t('pe.side.copied') : t('pe.side.copy')}
					</Button>
				</div>
				<div class="box__body">
					<div class="os">
						<label for="preview-os">{t('pe.side.os')}</label>
						<Select
							id="preview-os"
							options={[
								{ value: 'linux', label: 'Linux' },
								{ value: 'windows', label: 'Windows' },
								{ value: 'macos', label: 'macOS' }
							]}
							bind:value={os}
						/>
					</div>
					{#each steps as step, index (step.kind + index)}
						<div class="step">
							<div class="step__title">{step.title}</div>
							<pre
								class="step__cmd"
								class:step__cmd--script={step.kind === 'script'}>{step.command}</pre>
						</div>
					{/each}
					<p class="muted note">
						<Icon name="clock" size="xs" />
						{t('pe.side.limits', {
							stall:
								Number.parseInt(draft.stallMinutes, 10) > 0
									? draft.stallMinutes
									: STALL_DEFAULT_MINUTES,
							max:
								Number.parseInt(draft.maxDurationHours, 10) > 0
									? draft.maxDurationHours
									: MAX_DURATION_DEFAULT_HOURS
						})}
					</p>
				</div>
			</div>
		</aside>

		<div class="savebar">
			<span class="muted savebar__text">
				{#if !canWrite}
					{t('pe.save.readonly')}
				{:else}
					{dirty ? t('pe.save.dirty') : ''}
					{#if existing && planCount > 0}{t('pe.save.plans', { count: planCount })}{/if}
				{/if}
			</span>
			<Button onclick={() => goto(resolve('/(app)/profiles'))}>{t('common.cancel')}</Button>
			{#if canWrite}
				<Button type="submit" variant="primary" loading={saving}>{t('common.save')}</Button>
			{/if}
		</div>
	</form>
{/if}

<style>
	.layout {
		display: grid;
		grid-template-columns: minmax(0, 1fr) 340px;
		grid-template-areas:
			'main side'
			'bar bar';
		gap: var(--rh-space-4);
		align-items: start;
	}

	.main {
		display: flex;
		grid-area: main;
		flex-direction: column;
		gap: var(--rh-space-3);
		min-width: 0;
	}

	.side {
		position: sticky;
		top: var(--rh-space-4);
		display: flex;
		grid-area: side;
		flex-direction: column;
		gap: var(--rh-space-3);
		max-height: calc(100vh - 120px);
		overflow-y: auto;
	}

	.box {
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-lg);
		background-color: var(--rh-color-surface);
	}

	.box__head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--rh-space-2);
		padding: var(--rh-space-2) var(--rh-space-3);
		border-bottom: var(--rh-border-width) solid var(--rh-color-border-muted);
		font-weight: var(--rh-font-weight-semibold);
	}

	.box__body {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
		padding: var(--rh-space-3);
	}

	.result {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-2);
		margin: 0;
		padding: var(--rh-space-3);
		list-style: none;
		font-size: var(--rh-font-size-sm);
	}

	.result li {
		display: flex;
		align-items: flex-start;
		gap: var(--rh-space-2);
	}

	.result li .dot {
		margin-top: 6px;
	}

	.result__text {
		display: block;
		color: var(--rh-color-text-muted);
	}

	.dot {
		flex: none;
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background-color: var(--rh-color-border-strong);
	}

	.dot--on {
		background-color: var(--rh-color-success);
	}

	.dot--warn {
		background-color: var(--rh-color-warning);
	}

	.os {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
		font-size: var(--rh-font-size-sm);
	}

	.step__title {
		margin-bottom: var(--rh-space-1);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.step__cmd {
		margin: 0;
		padding: var(--rh-space-2);
		overflow-x: auto;
		border-radius: var(--rh-radius-sm);
		background-color: var(--rh-color-code-bg);
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-xs);
		white-space: pre-wrap;
		word-break: break-all;
	}

	.step__cmd--script {
		font-style: italic;
	}

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

	.warnings {
		margin: 0;
		padding-left: var(--rh-space-4);
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-sm);
	}

	.savebar {
		position: sticky;
		bottom: 0;
		z-index: var(--rh-z-sticky);
		display: flex;
		grid-area: bar;
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

	@media (max-width: 1000px) {
		.layout {
			grid-template-columns: minmax(0, 1fr);
			grid-template-areas:
				'main'
				'side'
				'bar';
		}

		.side {
			position: static;
			max-height: none;
		}
	}

	@media (max-width: 720px) {
		.grid2,
		.grid3 {
			grid-template-columns: minmax(0, 1fr);
		}
	}
</style>
