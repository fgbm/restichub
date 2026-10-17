<script lang="ts">
	import { onMount } from 'svelte';
	import { beforeNavigate, goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import { defaultPlanName, nameError } from '#lib/names.ts';
	import type {
		Agent,
		ApiProblem,
		CreatePlanRequest,
		MissedRunPolicy,
		Plan,
		Profile,
		Repository,
		UpdatePlanRequest
	} from '#lib/api/index.ts';
	import HostPicker from './HostPicker.svelte';
	import ProfilePicker from './ProfilePicker.svelte';
	import ProfileSection from './ProfileSection.svelte';
	import { RecordPage, SummaryPanel } from './layout/index.ts';
	import { Alert, Button, ConfirmDialog, Select, StatusPill, TextField } from './ui/index.ts';
	import type { KeyValueItem, SelectOption } from './ui/index.ts';
	import { formatLocalTime, formatTimeInZone } from '#lib/format.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import type { HostRef } from '#lib/host-picker.ts';
	import type { SummaryState } from '#lib/profile-form.ts';
	import {
		browserTimeZone,
		describeCron,
		describeMissedRunPolicy,
		timeZoneNames
	} from '#lib/schedule.ts';
	import { rememberSections } from '#lib/section-state.svelte.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	let { planId = null }: { planId?: string | null } = $props();

	type SectionKey = 'basics' | 'source' | 'schedule' | 'objective';
	const SECTIONS: SectionKey[] = ['source', 'basics', 'schedule', 'objective'];

	const POLICIES: MissedRunPolicy[] = ['run_all', 'latest_only', 'skip'];
	// The number of fire times that the preview requests (FR-SC-01).
	const PREVIEW_RUNS = 5;
	const TIME_ZONES = timeZoneNames();
	const TIME_ZONE_LIST_ID = 'plan-time-zones';
	const CRON_PRESETS: { label: MessageKey; cron: string }[] = [
		{ label: 'plans.preset.hourly', cron: '0 * * * *' },
		{ label: 'plans.preset.daily', cron: '0 2 * * *' },
		{ label: 'plans.preset.weekly', cron: '0 3 * * 0' }
	];

	let loading = $state(true);
	let loadError = $state<string | null>(null);
	let existing = $state<Plan | null>(null);
	let profiles = $state<Profile[]>([]);
	let agents = $state<Agent[]>([]);
	let repositories = $state<Repository[]>([]);
	let saving = $state(false);
	let saved = false;
	let deleteOpen = $state(false);
	let deleting = $state(false);
	let formError = $state<string | null>(null);
	let formProblem = $state<ApiProblem | null>(null);
	let baseline = $state('');
	const open = rememberSections(
		'plan',
		SECTIONS,
		Object.fromEntries(SECTIONS.map((key) => [key, true]))
	);

	let name = $state('');
	let nameEdited = $state(false);
	let planNames = $state<string[]>([]);
	let profileId = $state('');
	let agentId = $state('');
	let cron = $state('');
	let timeZone = $state('');
	let policy = $state<MissedRunPolicy>('run_all');
	let enabled = $state(true);
	// The objective in hours, as the person types it. Empty uses the schedule (ADR-0036).
	let rpoHours = $state('');

	let runs = $state<string[] | null>(null);
	let runsLoading = $state(false);

	const canWrite = $derived(['admin', 'operator'].includes(getSession()?.role ?? ''));
	const allOpen = $derived(SECTIONS.every((key) => open[key]));

	function repositoryOf(id: string): Repository | undefined {
		const repositoryId = profiles.find((profile) => profile.id === id)?.repository_id;
		return repositories.find((item) => item.id === repositoryId);
	}
	// A local path is on the disk of the repository host only, so the plan runs there (FR-RP-10).
	function localHostOf(item: Repository | undefined): string | null {
		return item?.backend.kind === 'local' ? (item.maintenance_agent_id ?? '') : null;
	}
	const repository = $derived(repositoryOf(profileId));
	const localHost = $derived(localHostOf(repository));
	// The stored host of a plan stays an option, so the field shows the real value and its error
	// instead of a silent move to another host.
	const hostChoices = $derived(
		agents.filter((agent) => agent.status === 'active' || agent.id === existing?.agent_id)
	);
	function hostReason(agent: HostRef): string | null {
		if (localHost === null || agent.id === localHost) return null;
		return localHost === ''
			? t('pl.source.localNoHost', { name: repositoryName })
			: t('hostPicker.localRepository', { repository: repositoryName, host: localHostName });
	}
	const hostMismatch = $derived(
		localHost !== null && agentId !== localHost ? t('reason.not_repository_host') : undefined
	);
	const policyOptions = $derived<SelectOption[]>(
		POLICIES.map((value) => ({ value, label: describeMissedRunPolicy(value, t) }))
	);

	const stateKey = $derived(
		JSON.stringify([name, profileId, agentId, cron, timeZone, policy, enabled, rpoHours])
	);
	const dirty = $derived(!loading && stateKey !== baseline);

	const profileName = $derived(profiles.find((profile) => profile.id === profileId)?.name ?? '');
	const hostName = $derived(agents.find((agent) => agent.id === agentId)?.hostname ?? '');
	const repositoryName = $derived(repository?.name ?? '');
	const localHostName = $derived(agents.find((agent) => agent.id === localHost)?.hostname ?? '');

	function selectProfile(id: string): void {
		const host = localHostOf(repositoryOf(id));
		if (host === null) return;
		const active = agents.some((agent) => agent.id === host && agent.status === 'active');
		agentId = active ? host : '';
	}

	function describeHours(seconds: number): string {
		const hours = seconds / 3600;
		return `${Number.isInteger(hours) ? hours : hours.toFixed(1)} h`;
	}

	const summaries = $derived<Record<SectionKey, { text: string; state: SummaryState }>>({
		basics: name ? { text: name, state: 'on' } : { text: t('pl.summary.noName'), state: 'warn' },
		source:
			profileName && hostName
				? { text: `${profileName} · ${hostName}`, state: 'on' }
				: { text: t('pl.summary.noSource'), state: 'warn' },
		schedule:
			cron.trim() && timeZone.trim()
				? { text: `${describeCron(cron, t)} · ${timeZone}`, state: 'on' }
				: { text: t('pl.summary.noSchedule'), state: 'warn' },
		objective: rpoHours.trim()
			? { text: `${rpoHours.trim()} h`, state: 'on' }
			: { text: t('pl.summary.rpoDefault'), state: 'off' }
	});

	const titles: Record<SectionKey, MessageKey> = {
		basics: 'pl.section.basics',
		source: 'pl.section.source',
		schedule: 'plans.section.schedule',
		objective: 'pl.section.objective'
	};

	const resultItems = $derived<KeyValueItem[]>([
		{ key: 'name', label: t('plans.plan'), value: name || t('pl.summary.noName') },
		{
			key: 'state',
			label: t('plans.state'),
			value: t(enabled ? 'plans.enabled' : 'plans.disabled')
		},
		{ key: 'profile', label: t('plans.profile'), value: profileName || t('common.none') },
		{ key: 'host', label: t('plans.host'), value: hostName || t('common.none') },
		...(repositoryName
			? [{ key: 'repository', label: t('pl.side.repository'), value: repositoryName }]
			: []),
		{ key: 'schedule', label: t('plans.schedule'), value: summaries.schedule.text },
		{ key: 'policy', label: t('plans.missedRunPolicy'), value: describeMissedRunPolicy(policy, t) },
		{
			key: 'rpo',
			label: t('plans.rpoColumn'),
			value: existing
				? `${summaries.objective.text} · ${t('plans.rpoEffective', { value: describeHours(existing.effective_rpo_seconds) })}`
				: summaries.objective.text
		},
		{
			key: 'next',
			label: t('plans.nextRun'),
			value: !enabled
				? t('plans.nextRunOff')
				: runs && runs.length > 0
					? formatTimeInZone(runs[0], timeZone)
					: t('common.none')
		}
	]);

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

	function utcTitle(value: string): string {
		return `${formatTimeInZone(value, 'UTC')} UTC · ${formatLocalTime(value)} ${t('common.localTime')}`;
	}

	async function load(): Promise<void> {
		try {
			const [planList, profileList, agentList, repositoryList] = await Promise.all([
				service.listPlans(),
				service.listProfiles(),
				service.listAgents(),
				// The repository is only a hint in the profile field and the summary, so its failure does not
				// block the form.
				service.listRepositories().catch(() => [])
			]);
			profiles = profileList;
			planNames = planList.map((item) => item.name);
			agents = agentList;
			repositories = repositoryList;
			if (planId) {
				const found = planList.find((item) => item.id === planId);
				if (!found) {
					loadError = t('pl.notFound');
					return;
				}
				existing = found;
				name = found.name;
				profileId = found.profile_id;
				agentId = found.agent_id;
				cron = found.cron;
				timeZone = found.time_zone;
				policy = found.missed_run_policy;
				enabled = found.enabled;
				rpoHours = found.rpo_seconds ? String(found.rpo_seconds / 3600) : '';
			} else {
				profileId = profiles[0]?.id ?? '';
				agentId = hostChoices.find((agent) => hostReason(agent) === null)?.id ?? '';
				timeZone = browserTimeZone();
				name = defaultPlanName(hostName, profileName, planNames);
			}
			baseline = stateKey;
		} catch (problem) {
			loadError = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	onMount(load);

	// A new plan gets a name from the host and the profile until the user types one.
	$effect(() => {
		if (loading || planId || nameEdited) return;
		name = defaultPlanName(hostName, profileName, planNames);
	});

	beforeNavigate(({ cancel }) => {
		if (dirty && !saved && !window.confirm(t('rp.unsaved'))) cancel();
	});

	// The page previews the next fire times from the cron and the time zone of the form, so the
	// user sees them before the save (FR-SC-01). The run list follows the edit of either field.
	$effect(() => {
		if (loading) return;
		const previewCron = cron;
		const previewTimeZone = timeZone;
		const handle = setTimeout(() => void loadRuns(previewCron, previewTimeZone), 200);
		return () => clearTimeout(handle);
	});

	async function loadRuns(previewCron: string, previewTimeZone: string): Promise<void> {
		if (!previewCron.trim() || !previewTimeZone.trim()) {
			runs = null;
			return;
		}
		runsLoading = true;
		try {
			const response = await service.previewSchedule({
				cron: previewCron,
				time_zone: previewTimeZone,
				count: PREVIEW_RUNS
			});
			runs = response.runs;
		} catch {
			runs = null;
		} finally {
			runsLoading = false;
		}
	}

	// A section that holds a server error opens, so the message is not hidden.
	$effect(() => {
		const field = formProblem?.field ?? '';
		const target: [string, SectionKey][] = [
			['name', 'basics'],
			['profile_id', 'source'],
			['agent_id', 'source'],
			['cron', 'schedule'],
			['time_zone', 'schedule'],
			['missed_run_policy', 'schedule'],
			['rpo_seconds', 'objective']
		];
		const hit = target.find(([prefix]) => field.startsWith(prefix));
		if (hit) open[hit[1]] = true;
	});

	function setAll(value: boolean): void {
		for (const key of SECTIONS) open[key] = value;
	}

	// An empty field gives null. Seconds above the limit of the API stay in the request, so the
	// control plane names the field in its answer.
	function rpoSeconds(): number | null {
		const hours = Number(rpoHours.replace(',', '.'));
		return rpoHours.trim() === '' || !Number.isFinite(hours) ? null : Math.round(hours * 3600);
	}

	async function save(): Promise<void> {
		saving = true;
		formError = null;
		formProblem = null;
		try {
			const body = {
				profile_id: profileId,
				agent_id: agentId,
				cron,
				time_zone: timeZone,
				missed_run_policy: policy,
				enabled,
				rpo_seconds: rpoSeconds()
			};
			if (existing) {
				await service.updatePlan(existing.id, { name, ...body } satisfies UpdatePlanRequest);
			} else {
				await service.createPlan({ name, ...body } satisfies CreatePlanRequest);
			}
			pushToast('success', t(existing ? 'plans.updated' : 'plans.created'));
			saved = true;
			await goto(resolve('/(app)/plans'));
		} catch (problem) {
			formProblem = problemOf(problem);
			formError = formProblem ? null : messageOf(problem);
		} finally {
			saving = false;
		}
	}

	// Delete is on the plan page only, so a list item keeps two actions.
	async function confirmDelete(): Promise<void> {
		if (!existing) return;
		deleting = true;
		try {
			await service.deletePlan(existing.id);
			pushToast('success', t('plans.deleted'));
			deleteOpen = false;
			saved = true;
			await goto(resolve('/(app)/plans'));
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			deleting = false;
		}
	}
</script>

<RecordPage
	title={planId ? (existing?.name ?? '') : t('pl.newTitle')}
	description={t(planId ? 'plans.editDescription' : 'plans.addDescription')}
	{loading}
	error={loadError}
	onsubmit={() => void save()}
	save={{
		dirty,
		saving,
		create: !planId,
		readonly: canWrite ? null : t('pl.save.readonly'),
		oncancel: () => goto(resolve('/(app)/plans'))
	}}
>
	{#snippet status()}
		{#if !loading && !enabled}<StatusPill>{t('plans.disabled')}</StatusPill>{/if}
	{/snippet}
	{#snippet actions()}
		<Button size="sm" onclick={() => setAll(true)} disabled={allOpen}>{t('pe.expandAll')}</Button>
		<Button size="sm" onclick={() => setAll(false)}>{t('pe.collapseAll')}</Button>
		{#if existing && canWrite}
			<Button
				size="sm"
				variant="ghost"
				icon="trash"
				class="btn--quiet-danger"
				title={t('plans.delete')}
				aria-label={t('plans.delete')}
				onclick={() => (deleteOpen = true)}
			/>
		{/if}
	{/snippet}
	{#snippet banners()}
		{#if formError}
			<Alert tone="danger" title={formError} />
		{/if}
	{/snippet}
	{#snippet side()}
		<SummaryPanel title={t('rp.side.result')} items={resultItems} />
	{/snippet}

	<ProfileSection
		id="source"
		title={t(titles.source)}
		summary={summaries.source.text}
		state={summaries.source.state}
		required
		bind:open={open.source}
	>
		<div class="grid2">
			<ProfilePicker
				id="plan-profile"
				{profiles}
				{repositories}
				{agents}
				error={fieldError(formProblem, 'profile_id')}
				bind:value={profileId}
				onchange={selectProfile}
				required
			/>
			<HostPicker
				id="plan-agent"
				hosts={hostChoices}
				reason={hostReason}
				error={fieldError(formProblem, 'agent_id') ?? hostMismatch}
				bind:value={agentId}
				required
			/>
		</div>
		{#if repositoryName && localHost === ''}
			<p class="muted">{t('pl.source.localNoHost', { name: repositoryName })}</p>
		{:else if repositoryName && localHost !== null}
			<p class="muted">
				{t('pl.source.localHost', { name: repositoryName, host: localHostName })}
			</p>
		{/if}
	</ProfileSection>

	<ProfileSection
		id="basics"
		title={t(titles.basics)}
		summary={summaries.basics.text}
		state={summaries.basics.state}
		required
		bind:open={open.basics}
	>
		<TextField
			id="plan-name"
			label={t('plans.plan')}
			help={t('names.help')}
			error={nameError(name) ?? fieldError(formProblem, 'name')}
			bind:value={name}
			oninput={() => (nameEdited = true)}
			required
		/>
	</ProfileSection>

	<ProfileSection
		id="schedule"
		title={t(titles.schedule)}
		summary={summaries.schedule.text}
		state={summaries.schedule.state}
		required
		switchable
		switchLabel={t('plans.runOnSchedule')}
		note={enabled ? undefined : t('plans.offNote')}
		bind:enabled
		bind:open={open.schedule}
	>
		<TextField
			id="plan-cron"
			mono
			label={t('plans.cron')}
			help={t('plans.cronHint')}
			error={fieldError(formProblem, 'cron')}
			placeholder={t('common.example', { value: '0 2 * * *' })}
			bind:value={cron}
			required
		/>
		<div class="presets" role="group" aria-label={t('plans.cronPresets')}>
			{#each CRON_PRESETS as preset (preset.cron)}
				<Button type="button" size="sm" onclick={() => (cron = preset.cron)}>
					{t(preset.label)}
				</Button>
			{/each}
		</div>
		<div class="grid2">
			<TextField
				id="plan-time-zone"
				label={t('plans.timeZone')}
				help={t('plans.timeZoneHint')}
				error={fieldError(formProblem, 'time_zone')}
				placeholder={t('plans.timeZonePlaceholder')}
				list={TIME_ZONE_LIST_ID}
				bind:value={timeZone}
				required
			/>
			<Select
				id="plan-policy"
				label={t('plans.missedRunPolicy')}
				options={policyOptions}
				error={fieldError(formProblem, 'missed_run_policy')}
				bind:value={policy}
			/>
		</div>
		<datalist id={TIME_ZONE_LIST_ID}>
			{#each TIME_ZONES as zone (zone)}
				<option value={zone}></option>
			{/each}
		</datalist>
		<div class="runs-block">
			<span class="muted">{t('plans.nextRuns')}</span>
			<div class="runs-body">
				{#if runsLoading && !runs}
					<p class="muted">{t('common.loading')}</p>
				{:else if runs && runs.length > 0}
					<ul class="runs">
						{#each runs as run (run)}
							<li title={utcTitle(run)}>{formatTimeInZone(run, timeZone)}</li>
						{/each}
					</ul>
				{:else}
					<p class="muted">{t('plans.previewEmpty')}</p>
				{/if}
			</div>
		</div>
	</ProfileSection>

	<ProfileSection
		id="objective"
		title={t(titles.objective)}
		summary={summaries.objective.text}
		state={summaries.objective.state}
		bind:open={open.objective}
	>
		<TextField
			id="plan-rpo"
			label={t('plans.rpo')}
			help={existing
				? `${t('plans.rpoHint')} ${t('plans.rpoEffective', { value: describeHours(existing.effective_rpo_seconds) })}`
				: t('plans.rpoHint')}
			error={fieldError(formProblem, 'rpo_seconds')}
			inputmode="decimal"
			placeholder={t('common.example', { value: '26' })}
			bind:value={rpoHours}
		/>
	</ProfileSection>

	<ConfirmDialog
		bind:open={deleteOpen}
		title={t('plans.deleteTitle')}
		message={t('plans.deleteBody', { name: existing?.name ?? '' })}
		confirmLabel={t('plans.delete')}
		tone="danger"
		loading={deleting}
		onConfirm={confirmDelete}
	/>
</RecordPage>

<style>
	.muted {
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.grid2 {
		display: grid;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		gap: var(--rh-space-3);
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

	/* The preview arrives after a delay. The reserved height keeps the sections below it in place. */
	.runs-body {
		min-height: calc(var(--rh-font-size-sm) * var(--rh-line-height-base) * 5 + 2px * 4);
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

	@media (max-width: 720px) {
		.grid2 {
			grid-template-columns: minmax(0, 1fr);
		}
	}
</style>
