<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import { latest } from '#lib/latest.ts';
	import { defaultPlanName, nameError } from '#lib/names.ts';
	import type {
		Agent,
		ApiProblem,
		CreatePlanRequest,
		HostParameterRow,
		MissedRunPolicy,
		ParameterKind,
		ParameterValueDef,
		Plan,
		PlanParameterValue,
		Profile,
		Repository,
		Secret,
		UpdatePlanRequest
	} from '#lib/api/index.ts';
	import HostPicker from './HostPicker.svelte';
	import ProfilePicker from './ProfilePicker.svelte';
	import RepositoryPicker from './RepositoryPicker.svelte';
	import ProfileSection from './ProfileSection.svelte';
	import { RecordPage, SummaryPanel } from './layout/index.ts';
	import {
		Alert,
		Badge,
		Button,
		ConfirmDialog,
		DangerZone,
		DataTable,
		Dialog,
		Icon,
		Select,
		StatusPill,
		TextField
	} from './ui/index.ts';
	import type { DangerAction, KeyValueItem, SelectOption, TableColumn } from './ui/index.ts';
	import { formatHours, formatLocalTime, formatTimeInZone } from '#lib/format.ts';
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
	import {
		canWriteSecret,
		kindLabel,
		missingValue,
		parameterCount,
		resolveValue
	} from '#lib/parameters.ts';
	import type { ResolvedValue, ValueSource } from '#lib/parameters.ts';
	import { plural } from '#lib/list-format.ts';

	let { planId = null }: { planId?: string | null } = $props();

	type SectionKey = 'basics' | 'source' | 'parameters' | 'schedule' | 'objective' | 'danger';
	const SECTIONS: SectionKey[] = [
		'source',
		'parameters',
		'basics',
		'schedule',
		'objective',
		'danger'
	];

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
	let secrets = $state<Secret[]>([]);
	/** The values on the host of the plan, as the server shows them to this reader (FR-PF-12). */
	let hostParameters = $state<HostParameterRow[]>([]);
	/** The override that the plan sets, by parameter name. It is never sent for a name that the profile drops. */
	let planValues = $state<Record<string, { kind: ParameterKind; text: string; secretId: string }>>(
		{}
	);
	let valueDialog = $state(false);
	let valueParameter = $state('');
	let valueText = $state('');
	let valueSecretId = $state('');
	let saving = $state(false);
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
	let repositoryId = $state('');
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
		return repositories.find((item) => item.id === id);
	}
	// A local path is on the disk of the repository host only, so the plan runs there (FR-RP-10).
	function localHostOf(item: Repository | undefined): string | null {
		return item?.backend.kind === 'local' ? (item.maintenance_agent_id ?? '') : null;
	}
	const repository = $derived(repositoryOf(repositoryId));
	const localHost = $derived(localHostOf(repository));
	// A stored host of a plan stays an option, so the field shows the real value and its error
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
		JSON.stringify([
			name,
			profileId,
			repositoryId,
			agentId,
			cron,
			timeZone,
			policy,
			enabled,
			rpoHours,
			planValues
		])
	);
	const dirty = $derived(!loading && stateKey !== baseline);

	const profileName = $derived(profiles.find((profile) => profile.id === profileId)?.name ?? '');
	const hostName = $derived(agents.find((agent) => agent.id === agentId)?.hostname ?? '');
	const repositoryName = $derived(repository?.name ?? '');
	const localHostName = $derived(agents.find((agent) => agent.id === localHost)?.hostname ?? '');

	// The host field follows the repository: a local path runs on one host (AC-RP-06, AC-RP-07).
	function selectRepository(id: string): void {
		const host = localHostOf(repositoryOf(id));
		if (host === null) return;
		const active = agents.some((agent) => agent.id === host && agent.status === 'active');
		agentId = active ? host : '';
	}

	/* ------------------------------------------------------- profile parameters */

	const profile = $derived(profiles.find((item) => item.id === profileId));
	const declared = $derived(profile?.parameters ?? []);
	const canSetSecret = $derived(canWriteSecret(getSession()?.role));
	const secretOptions = $derived<SelectOption[]>([
		{ value: '', label: t('scripts.parameter.selectSecret') },
		...secrets.map((secret) => ({ value: secret.id, label: secret.name }))
	]);

	// The plan sets a value for a name of a declared parameter only, so a stale edit is dropped.
	const planParameterValues = $derived<PlanParameterValue[]>(
		declared.flatMap<PlanParameterValue>((definition) => {
			const value = planValues[definition.name];
			if (!value || value.kind !== definition.kind) return [];
			if (definition.kind === 'secret') {
				if (value.secretId === '') return [];
				return [
					{
						name: definition.name,
						kind: 'secret' as const,
						value: null,
						secret_id: value.secretId,
						secret_name: secrets.find((item) => item.id === value.secretId)?.name ?? null,
						hidden: false
					}
				];
			}
			if (value.text.trim() === '') return [];
			return [
				{
					name: definition.name,
					kind: 'text' as const,
					value: value.text,
					secret_id: null,
					secret_name: null,
					hidden: false
				}
			];
		})
	);
	const resolved = $derived(
		Object.fromEntries(
			declared.map((definition) => [
				definition.name,
				resolveValue(definition, planParameterValues, hostParameters)
			])
		)
	);
	const missing = $derived(
		declared.filter((definition) => missingValue(definition, planParameterValues, hostParameters))
	);
	// A refusal of the server names the parameters of its own check, for example a value of another
	// kind, so the section shows that names too (FR-PF-11).
	const serverMissing = $derived(
		formProblem?.code === 'parameter_missing' ? (formProblem.parameters ?? []).join(', ') : null
	);

	const blockedSave = $derived(
		missing.length > 0 ? t('pl.param.missingBlocked', { name: missing[0].name }) : null
	);

	const parameterSummary = $derived.by<{ text: string; state: SummaryState }>(() => {
		if (declared.length === 0) return { text: t('pl.param.noParameters'), state: 'off' };
		if (missing.length > 0) {
			return { text: plural('pl.param.missingSummary', missing.length), state: 'warn' };
		}
		return { text: parameterCount(declared.length), state: 'on' };
	});

	/** A row of the table: the declaration and the value that the server would use. */
	type ParameterRow = Profile['parameters'][number] & { resolved: ResolvedValue };

	// A new row object follows a change of the value, so the row actions follow it too.
	const parameterRows = $derived<ParameterRow[]>(
		declared.map((definition) => ({
			...definition,
			resolved: resolved[definition.name] ?? {
				source: 'none' as ValueSource,
				text: '',
				secretName: null,
				hidden: false
			}
		}))
	);

	function sourceLabel(source: ValueSource): string {
		return t(`pl.param.source.${source}` as MessageKey);
	}

	// A value is set for the whole plan, so the row asks for the value of one parameter.
	function openValueDialog(name: string): void {
		const value = planValues[name];
		valueParameter = name;
		valueText = value?.text ?? '';
		valueSecretId = value?.secretId ?? '';
		valueDialog = true;
	}

	function setPlanValue(): void {
		const definition = declared.find((item) => item.name === valueParameter);
		if (!definition) return;
		planValues = {
			...planValues,
			[definition.name]: {
				kind: definition.kind,
				text: definition.kind === 'text' ? valueText : '',
				secretId: definition.kind === 'secret' ? valueSecretId : ''
			}
		};
		valueDialog = false;
	}

	// The values of a host are one table on the host page, so this action goes there and does not
	// copy that table into a dialog (UI principles 1.5, FR-PF-10).
	async function openHostParameters(): Promise<void> {
		if (!agentId) return;
		// The section of the host page is the anchor of the address.
		// eslint-disable-next-line svelte/no-navigation-without-resolve -- resolved path with an anchor
		await goto(`${resolve('/(app)/hosts/[id]', { id: agentId })}#parameters`);
	}

	function removePlanValue(name: string): void {
		const { [name]: _removed, ...rest } = planValues;
		planValues = rest;
	}

	// A host that the person leaves behind answers late, so the table follows the newest request.
	const hostParameterRequests = latest();

	// The host values belong to the host of the plan, so they follow the host field.
	async function loadHostParameters(): Promise<void> {
		const current = hostParameterRequests.start();
		if (!agentId) {
			hostParameters = [];
			return;
		}
		try {
			const response = await service.getAgentParameters(agentId);
			if (!current()) return;
			hostParameters = response.parameters;
		} catch {
			if (!current()) return;
			// The section shows the plan values and the defaults; a missing host value stays "not set".
			hostParameters = [];
		}
	}

	$effect(() => {
		if (loading) return;
		const id = agentId;
		void id;
		void loadHostParameters();
	});

	const summaries = $derived<Record<SectionKey, { text: string; state: SummaryState }>>({
		parameters: parameterSummary,
		basics: name ? { text: name, state: 'on' } : { text: t('pl.summary.noName'), state: 'warn' },
		source:
			profileName && repositoryName && hostName
				? { text: `${profileName} · ${repositoryName} · ${hostName}`, state: 'on' }
				: { text: t('pl.summary.noSource'), state: 'warn' },
		schedule:
			cron.trim() && timeZone.trim()
				? { text: `${describeCron(cron, t)} · ${timeZone}`, state: 'on' }
				: { text: t('pl.summary.noSchedule'), state: 'warn' },
		objective: rpoHours.trim()
			? { text: t('duration.h', { h: rpoHours.trim() }), state: 'on' }
			: { text: t('pl.summary.rpoDefault'), state: 'off' },
		danger: { text: t('pl.danger.summary'), state: 'off' }
	});

	const titles: Record<SectionKey, MessageKey> = {
		basics: 'pl.section.basics',
		source: 'pl.section.source',
		parameters: 'pl.section.parameters',
		schedule: 'plans.section.schedule',
		objective: 'pl.section.objective',
		danger: 'danger.title'
	};

	const resultItems = $derived<KeyValueItem[]>([
		{ key: 'name', label: t('plans.plan'), value: name || t('pl.summary.noName') },
		{
			key: 'state',
			label: t('plans.state'),
			value: t(enabled ? 'plans.enabled' : 'plans.disabled')
		},
		{ key: 'profile', label: t('plans.profile'), value: profileName || t('common.none') },
		{
			key: 'repository',
			label: t('pl.side.repository'),
			value: repositoryName || t('common.none')
		},
		{ key: 'host', label: t('plans.host'), value: hostName || t('common.none') },
		{ key: 'schedule', label: t('plans.schedule'), value: summaries.schedule.text },
		{ key: 'policy', label: t('plans.missedRunPolicy'), value: describeMissedRunPolicy(policy, t) },
		{
			key: 'rpo',
			label: t('plans.rpoColumn'),
			value: existing
				? `${summaries.objective.text} · ${t('plans.rpoEffective', { value: formatHours(existing.effective_rpo_seconds) })}`
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

	// The delete removes the secret override of the plan, and only an Admin removes a secret value
	// (ADR-0047).
	const secretOverride = $derived(
		existing?.parameter_values.some((value) => value.kind === 'secret') ?? false
	);

	// Delete is on the plan page only, so a list item keeps two actions.
	const dangerActions = $derived.by<DangerAction[]>(() => {
		if (!existing || !canWrite) return [];
		const refusedSecret = secretOverride && !canSetSecret;
		return [
			{
				key: 'delete',
				text: t('pl.danger.delete'),
				label: t('plans.delete'),
				icon: 'trash',
				tone: 'danger',
				disabled: refusedSecret,
				hint: refusedSecret ? t('pl.danger.deleteSecret') : undefined,
				onclick: () => (deleteOpen = true)
			}
		];
	});

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
			const [planList, profileList, agentList, repositoryList, secretList] = await Promise.all([
				service.listPlans(),
				service.listProfiles(),
				service.listAgents(),
				// The repository is only a hint in the profile field and the summary, so its failure does not
				// block the form.
				service.listRepositories().catch(() => []),
				service.listSecrets().catch(() => [])
			]);
			secrets = secretList;
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
				repositoryId = found.repository_id;
				agentId = found.agent_id;
				cron = found.cron;
				timeZone = found.time_zone;
				policy = found.missed_run_policy;
				enabled = found.enabled;
				rpoHours = found.rpo_seconds ? String(found.rpo_seconds / 3600) : '';
				planValues = Object.fromEntries(
					found.parameter_values.map((value) => [
						value.name,
						{
							kind: value.kind,
							text: value.kind === 'text' ? (value.value ?? '') : '',
							secretId: value.secret_id ?? ''
						}
					])
				);
			} else {
				profileId = profiles[0]?.id ?? '';
				// A repository without an init step holds no snapshot, so the form prefers another.
				repositoryId =
					repositories.find((item) => item.initialized)?.id ?? repositories[0]?.id ?? '';
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

	// The page previews the next fire times from the cron and the time zone of the form, so the
	// user sees them before the save (FR-SC-01). The run list follows the edit of either field.
	$effect(() => {
		if (loading) return;
		const previewCron = cron;
		const previewTimeZone = timeZone;
		const handle = setTimeout(() => void loadRuns(previewCron, previewTimeZone), 200);
		return () => clearTimeout(handle);
	});

	// A typed cron answers late, so the list follows the newest request of the preview.
	const runRequests = latest();

	async function loadRuns(previewCron: string, previewTimeZone: string): Promise<void> {
		const current = runRequests.start();
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
			if (!current()) return;
			runs = response.runs;
		} catch {
			if (!current()) return;
			runs = null;
		} finally {
			if (current()) runsLoading = false;
		}
	}

	// A section that holds a server error opens, so the message is not hidden.
	$effect(() => {
		const field = formProblem?.field ?? '';
		const target: [string, SectionKey][] = [
			['name', 'basics'],
			['profile_id', 'source'],
			['repository_id', 'source'],
			['agent_id', 'source'],
			['cron', 'schedule'],
			['time_zone', 'schedule'],
			['missed_run_policy', 'schedule'],
			['rpo_seconds', 'objective'],
			['parameters', 'parameters']
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
				repository_id: repositoryId,
				agent_id: agentId,
				cron,
				time_zone: timeZone,
				missed_run_policy: policy,
				enabled,
				rpo_seconds: rpoSeconds(),
				parameter_values: planParameterValues.map((value): ParameterValueDef => ({
					name: value.name,
					kind: value.kind,
					...(value.kind === 'secret'
						? { secret_id: value.secret_id ?? undefined }
						: { value: value.value ?? '' })
				}))
			};
			if (existing) {
				await service.updatePlan(existing.id, { name, ...body } satisfies UpdatePlanRequest);
			} else {
				await service.createPlan({ name, ...body } satisfies CreatePlanRequest);
			}
			pushToast('success', t(existing ? 'plans.updated' : 'plans.created'));
			baseline = stateKey;
			await goto(resolve('/(app)/plans'));
		} catch (problem) {
			formProblem = problemOf(problem);
			formError = formProblem ? null : messageOf(problem);
		} finally {
			saving = false;
		}
	}

	async function confirmDelete(): Promise<void> {
		if (!existing) return;
		deleting = true;
		try {
			await service.deletePlan(existing.id);
			pushToast('success', t('plans.deleted'));
			deleteOpen = false;
			baseline = stateKey;
			await goto(resolve('/(app)/plans'));
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			deleting = false;
		}
	}
</script>

{#snippet planParameterNameCell(row: ParameterRow)}
	<span class="mono nowrap">{row.name}</span>
	<span class="param-kind">
		{#if row.kind === 'secret'}<Icon name="lock" size="xs" />{/if}
		{kindLabel(row.kind)}{#if row.required}<span class="t-danger">
				· {t('pl.param.required')}</span
			>{/if}
	</span>
{/snippet}

{#snippet planParameterValueCell(row: ParameterRow)}
	{#if row.resolved.source === 'none'}
		<span class="value value--missing">{t('common.notSet')}</span>
	{:else if row.kind === 'secret'}
		<span class="value"
			><span class="mask">••••••</span>
			<span class="muted">{row.resolved.secretName ?? ''}</span></span
		>
	{:else}
		<span class="value mono">{row.resolved.text}</span>
	{/if}
{/snippet}

{#snippet planParameterSourceCell(row: ParameterRow)}
	{@const source = row.resolved.source}
	<Badge
		tone={source === 'plan'
			? 'accent'
			: source === 'host'
				? 'info'
				: source === 'none'
					? 'danger'
					: 'neutral'}
	>
		{sourceLabel(source)}
	</Badge>
{/snippet}

<RecordPage
	title={planId ? (existing?.name ?? '') : t('pl.newTitle')}
	description={t('plans.description')}
	{loading}
	error={loadError}
	onsubmit={() => void save()}
	save={{
		dirty,
		saving,
		create: !planId,
		readonly: canWrite ? null : t('pl.save.readonly'),
		blocked: blockedSave,
		oncancel: () => goto(resolve('/(app)/plans'))
	}}
>
	{#snippet status()}
		{#if !loading && !enabled}<StatusPill>{t('plans.disabled')}</StatusPill>{/if}
	{/snippet}
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
				error={fieldError(formProblem, 'profile_id')}
				bind:value={profileId}
				required
			/>
			<RepositoryPicker
				id="plan-repository"
				{repositories}
				{agents}
				error={fieldError(formProblem, 'repository_id')}
				bind:value={repositoryId}
				onchange={selectRepository}
				required
			/>
		</div>
		<HostPicker
			id="plan-agent"
			hosts={hostChoices}
			reason={hostReason}
			error={fieldError(formProblem, 'agent_id') ?? hostMismatch}
			bind:value={agentId}
			required
		/>
		{#if repositoryName && localHost === ''}
			<p class="muted">{t('pl.source.localNoHost', { name: repositoryName })}</p>
		{:else if repositoryName && localHost !== null}
			<p class="muted">
				{t('pl.source.localHost', { name: repositoryName, host: localHostName })}
			</p>
		{/if}
	</ProfileSection>

	<ProfileSection
		id="parameters"
		title={t(titles.parameters)}
		summary={summaries.parameters.text}
		state={summaries.parameters.state}
		badge={declared.length > 0 ? String(declared.length) : undefined}
		bind:open={open.parameters}
	>
		<p class="lead">{t('pl.param.hint')}</p>
		{#if missing.length > 0}
			<Alert tone="danger" title={t('pl.param.missingTitle', { name: missing[0].name })}>
				{t('pl.param.missingHint', { host: hostName })}
			</Alert>
		{/if}
		{#if serverMissing}
			<Alert
				variant="inline"
				tone="danger"
				title={t('pl.param.missingTitle', { name: serverMissing })}
			/>
		{:else if fieldError(formProblem, 'parameters')}
			<Alert variant="inline" tone="danger" title={fieldError(formProblem, 'parameters') ?? ''} />
		{/if}
		{#if declared.length === 0}
			<p class="muted">{t('pl.param.noParameters')}</p>
		{:else}
			<DataTable
				paginated={false}
				rowKey={(row) => row.name}
				rows={parameterRows}
				columns={[
					{
						key: 'name',
						header: t('parameters.name'),
						sortable: false,
						nowrap: true,
						render: planParameterNameCell
					},
					{
						key: 'value',
						header: t('pl.param.value'),
						sortable: false,
						render: planParameterValueCell
					},
					{
						key: 'source',
						header: t('pl.param.source'),
						sortable: false,
						nowrap: true,
						render: planParameterSourceCell
					}
				] satisfies TableColumn<ParameterRow>[]}
				actions={(row) => {
					const source = row.resolved.source;
					const secretLocked = row.kind === 'secret' && !canSetSecret;
					return [
						{
							id: 'plan',
							label: t('pl.param.setForPlanOf', { name: row.name }),
							hint: secretLocked ? t('pl.param.secretLocked') : t('pl.param.setForPlan'),
							icon: 'replace',
							disabled: !canWrite || secretLocked,
							onSelect: () => openValueDialog(row.name)
						},
						{
							id: 'host',
							label: t('pl.param.setOnHostOf', { name: row.name }),
							hint: t('pl.param.setOnHost'),
							icon: 'host',
							// A value of the plan hides the host value, so the host field would mislead.
							hidden: row.kind === 'secret' || source === 'plan',
							disabled: !canWrite || !agentId,
							onSelect: () => void openHostParameters()
						},
						{
							id: 'remove',
							label: t('pl.param.removePlanOf', { name: row.name }),
							hint: t('pl.param.removePlan'),
							icon: 'rotate-ccw',
							hidden: source !== 'plan',
							disabled: !canWrite,
							onSelect: () => removePlanValue(row.name)
						}
					];
				}}
			/>
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
				? `${t('plans.rpoHint')} ${t('plans.rpoEffective', { value: formatHours(existing.effective_rpo_seconds) })}`
				: t('plans.rpoHint')}
			error={fieldError(formProblem, 'rpo_seconds')}
			inputmode="decimal"
			placeholder={t('common.example', { value: '26' })}
			bind:value={rpoHours}
		/>
	</ProfileSection>

	{#if dangerActions.length > 0}
		<ProfileSection
			id="danger"
			title={t(titles.danger)}
			summary={summaries.danger.text}
			state={summaries.danger.state}
			bind:open={open.danger}
		>
			<DangerZone actions={dangerActions} />
		</ProfileSection>
	{/if}

	<Dialog
		bind:open={valueDialog}
		size="md"
		title={t('pl.param.dialogTitle', { name: valueParameter })}
		description={t('pl.param.dialogDescription')}
		onsubmit={setPlanValue}
	>
		{#if declared.find((item) => item.name === valueParameter)?.kind === 'secret'}
			<Select
				label={t('pl.param.dialogSecret')}
				options={secretOptions}
				bind:value={valueSecretId}
			/>
		{:else}
			<TextField label={t('pl.param.value')} mono bind:value={valueText} />
		{/if}
		{#snippet footer()}
			<Button onclick={() => (valueDialog = false)}>{t('common.cancel')}</Button>
			<Button type="submit" variant="primary">{t('pl.param.dialogSet')}</Button>
		{/snippet}
	</Dialog>

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
	.param-kind {
		display: flex;
		align-items: center;
		gap: var(--rh-space-1);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-xs);
	}

	.value {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-2);
		overflow-wrap: anywhere;
	}

	.value--missing {
		color: var(--rh-color-danger-text);
	}

	.mask {
		font-family: var(--rh-font-mono);
		letter-spacing: 1px;
	}

	.t-danger {
		color: var(--rh-color-danger-text);
	}

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
