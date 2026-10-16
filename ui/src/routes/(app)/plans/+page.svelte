<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import type {
		ApiProblem,
		CreatePlanRequest,
		MissedRunPolicy,
		Plan,
		UpdatePlanRequest
	} from '#lib/api/index.ts';
	import {
		Alert,
		Badge,
		Button,
		Checkbox,
		ConfirmDialog,
		Dialog,
		EmptyState,
		PageHeader,
		Select,
		Table,
		TextField,
		RowActions
	} from '#lib/components/ui/index.ts';
	import type { MenuItem, SelectOption, TableColumn } from '#lib/components/ui/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { formatTimeInZoneWithLocal } from '#lib/format.ts';
	import {
		browserTimeZone,
		describeCron,
		describeMissedRunPolicy,
		timeZoneNames
	} from '#lib/schedule.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	const POLICIES: MissedRunPolicy[] = ['run_all', 'latest_only', 'skip'];
	// The number of fire times that the dialog preview requests (FR-SC-01).
	const PREVIEW_RUNS = 5;
	// The IANA names of the searchable time zone input, and its datalist identifier.
	const TIME_ZONES = timeZoneNames();
	const TIME_ZONE_LIST_ID = 'plan-time-zones';
	// The quick cron presets of the dialog (FR-SC-01).
	const CRON_PRESETS: { label: MessageKey; cron: string }[] = [
		{ label: 'plans.preset.hourly', cron: '0 * * * *' },
		{ label: 'plans.preset.daily', cron: '0 2 * * *' },
		{ label: 'plans.preset.weekly', cron: '0 3 * * 0' }
	];

	let plans = $state<Plan[]>([]);
	let profilesOptions = $state<SelectOption[]>([]);
	let agentOptions = $state<SelectOption[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let formOpen = $state(false);
	let editing = $state<Plan | null>(null);
	let saving = $state(false);
	let formError = $state<string | null>(null);
	let formProblem = $state<ApiProblem | null>(null);

	let name = $state('');
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

	let deleteTarget = $state<Plan | null>(null);
	let deleteOpen = $state(false);
	let deleting = $state(false);

	const canWrite = $derived(['admin', 'operator'].includes(getSession()?.role ?? ''));

	const policyOptions = $derived<SelectOption[]>(
		POLICIES.map((value) => ({ value, label: describeMissedRunPolicy(value, t) }))
	);

	const columns = $derived<TableColumn<Plan>[]>([
		{ key: 'name', header: t('plans.plan'), value: (row) => row.name, render: nameCell },
		{
			key: 'host',
			header: t('plans.host'),
			value: (row) => row.agent_hostname,
			render: hostCell
		},
		{ key: 'profile', header: t('plans.profile'), value: (row) => row.profile_name },
		{
			key: 'schedule',
			header: t('plans.schedule'),
			value: (row) => describeCron(row.cron, t),
			render: scheduleCell
		},
		{ key: 'time_zone', header: t('plans.timeZone'), value: (row) => row.time_zone },
		{
			key: 'next_run_at',
			nowrap: true,
			header: t('plans.nextRun'),
			value: (row) =>
				row.next_run_at ? formatTimeInZoneWithLocal(row.next_run_at, row.time_zone) : '',
			render: nextRunCell
		},
		{
			key: 'missed_run_policy',
			header: t('plans.missedRunPolicy'),
			value: (row) => describeMissedRunPolicy(row.missed_run_policy, t)
		}
	]);

	function describeHours(seconds: number): string {
		const hours = seconds / 3600;
		return `${Number.isInteger(hours) ? hours : hours.toFixed(1)} h`;
	}

	const empty = $derived(!loading && !error && plans.length === 0);

	function actionsFor(plan: Plan): MenuItem[] {
		const items: MenuItem[] = [];
		if (canWrite) {
			items.push({ label: t('plans.run'), icon: 'activity', onSelect: () => void runNow(plan) });
			items.push({ label: t('plans.edit'), icon: 'settings', onSelect: () => openEdit(plan) });
			items.push({
				label: t('plans.delete'),
				icon: 'octagon',
				tone: 'danger',
				onSelect: () => {
					deleteTarget = plan;
					deleteOpen = true;
				}
			});
		}
		return items;
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
			const [planList, profileList, agentList] = await Promise.all([
				service.listPlans(),
				service.listProfiles(),
				service.listAgents()
			]);
			plans = planList;
			profilesOptions = profileList.map((profile) => ({
				value: profile.id,
				label: profile.name
			}));
			agentOptions = agentList
				.filter((agent) => agent.status === 'active')
				.map((agent) => ({ value: agent.id, label: agent.hostname }));
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

	// The dialog previews the next fire times from the cron and the time zone of the form, so the
	// user sees them before the save (FR-SC-01). The run list follows the edit of either field.
	$effect(() => {
		if (!formOpen) return;
		const previewCron = cron;
		const previewTimeZone = timeZone;
		const handle = setTimeout(() => void loadRuns(previewCron, previewTimeZone), 200);
		return () => clearTimeout(handle);
	});

	function resetForm(): void {
		name = '';
		profileId = '';
		agentId = '';
		cron = '';
		timeZone = '';
		policy = 'run_all';
		enabled = true;
		rpoHours = '';
		runs = null;
		runsLoading = false;
		formError = null;
		formProblem = null;
	}

	function openCreate(): void {
		editing = null;
		resetForm();
		profileId = profilesOptions[0]?.value ?? '';
		agentId = agentOptions[0]?.value ?? '';
		timeZone = browserTimeZone();
		formOpen = true;
	}

	function openEdit(plan: Plan): void {
		editing = plan;
		resetForm();
		name = plan.name;
		profileId = plan.profile_id;
		agentId = plan.agent_id;
		cron = plan.cron;
		timeZone = plan.time_zone;
		policy = plan.missed_run_policy;
		enabled = plan.enabled;
		rpoHours = plan.rpo_seconds ? String(plan.rpo_seconds / 3600) : '';
		formOpen = true;
	}

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
			if (editing) {
				await service.updatePlan(editing.id, { name, ...body } satisfies UpdatePlanRequest);
			} else {
				await service.createPlan({ name, ...body } satisfies CreatePlanRequest);
			}
			pushToast('success', t(editing ? 'plans.updated' : 'plans.created'));
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

	async function runNow(plan: Plan): Promise<void> {
		try {
			await service.runPlan(plan.id);
			pushToast('success', t('plans.runQueued'));
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		}
	}

	async function confirmDelete(): Promise<void> {
		if (!deleteTarget) return;
		deleting = true;
		try {
			await service.deletePlan(deleteTarget.id);
			pushToast('success', t('plans.deleted'));
			deleteOpen = false;
			deleteTarget = null;
			await load();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			deleting = false;
		}
	}
</script>

{#snippet nameCell(plan: Plan)}
	<span class="plan">
		<span class="plan__name">{plan.name}</span>
		{#if !plan.enabled}
			<Badge tone="neutral">{t('plans.disabled')}</Badge>
		{/if}
	</span>
{/snippet}

{#snippet hostCell(plan: Plan)}
	<span class="plan__host">{plan.agent_hostname}</span>
{/snippet}

{#snippet scheduleCell(plan: Plan)}
	<span class="plan">
		<span>{describeCron(plan.cron, t)}</span>
		<span class="plan__meta plan__cron">{plan.cron}</span>
	</span>
{/snippet}

{#snippet nextRunCell(plan: Plan)}
	<span class="plan__meta">
		{plan.next_run_at
			? formatTimeInZoneWithLocal(plan.next_run_at, plan.time_zone)
			: t('common.none')}
	</span>
{/snippet}

<svelte:head>
	<title>{t('plans.title')} · {t('app.name')}</title>
</svelte:head>

<PageHeader
	title={t('plans.title')}
	description={t('plans.description')}
	count={loading ? undefined : plans.length}
>
	{#if canWrite}
		<Button variant="primary" icon="plus" onclick={openCreate}>{t('plans.add')}</Button>
	{/if}
</PageHeader>

{#if empty}
	<EmptyState
		icon="calendar"
		title={t('plans.emptyTitle')}
		description={t('plans.emptyDescription')}
		bordered
	>
		{#if canWrite}
			<Button variant="primary" onclick={openCreate}>{t('plans.addFirst')}</Button>
		{/if}
	</EmptyState>
{:else}
	<Table
		{columns}
		rows={plans}
		{loading}
		{error}
		rowKey={(plan) => plan.id}
		filterPlaceholder={t('plans.searchPlaceholder')}
	>
		{#snippet rowActions(plan)}
			{#if actionsFor(plan).length > 0}
				<RowActions
					moreLabel={t('plans.moreActions', { name: plan.name })}
					items={actionsFor(plan)}
				/>
			{/if}
		{/snippet}
	</Table>
{/if}

<Dialog
	bind:open={formOpen}
	size="lg"
	title={editing ? t('plans.editTitle') : t('plans.addTitle')}
	description={editing ? t('plans.editDescription') : t('plans.addDescription')}
	onsubmit={save}
>
	{#if formError}
		<Alert tone="danger" title={formError} />
	{/if}
	<div class="form">
		<section class="section">
			<h3 class="section__title">{t('plans.section.source')}</h3>
			<TextField
				id="plan-name"
				label={t('plans.plan')}
				error={fieldError(formProblem, 'name')}
				bind:value={name}
				required
			/>
			<Select
				id="plan-profile"
				label={t('plans.profile')}
				options={profilesOptions}
				error={fieldError(formProblem, 'profile_id')}
				bind:value={profileId}
				required
			/>
			<Select
				id="plan-agent"
				label={t('plans.host')}
				options={agentOptions}
				error={fieldError(formProblem, 'agent_id')}
				bind:value={agentId}
				required
			/>
		</section>

		<section class="section">
			<h3 class="section__title">{t('plans.section.schedule')}</h3>
			<TextField
				id="plan-cron"
				label={t('plans.cron')}
				help={t('plans.cronHint')}
				error={fieldError(formProblem, 'cron')}
				placeholder="0 2 * * *"
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
			<datalist id={TIME_ZONE_LIST_ID}>
				{#each TIME_ZONES as zone (zone)}
					<option value={zone}></option>
				{/each}
			</datalist>
			<Select
				id="plan-policy"
				label={t('plans.missedRunPolicy')}
				options={policyOptions}
				bind:value={policy}
			/>
			<TextField
				id="plan-rpo"
				label={t('plans.rpo')}
				help={editing
					? `${t('plans.rpoHint')} ${t('plans.rpoEffective', { value: describeHours(editing.effective_rpo_seconds) })}`
					: t('plans.rpoHint')}
				error={fieldError(formProblem, 'rpo_seconds')}
				inputmode="decimal"
				placeholder="26"
				bind:value={rpoHours}
			/>
			<Checkbox label={t('plans.enabled')} bind:checked={enabled} />
		</section>

		<section class="section">
			<h3 class="section__title">{t('plans.nextRuns')}</h3>
			{#if runsLoading}
				<p class="preview__hint">{t('common.loading')}</p>
			{:else if runs && runs.length > 0}
				<ul class="runs">
					{#each runs as run (run)}
						<li>{formatTimeInZoneWithLocal(run, timeZone)}</li>
					{/each}
				</ul>
			{:else}
				<p class="preview__hint">{t('plans.previewEmpty')}</p>
			{/if}
		</section>
	</div>
	{#snippet footer()}
		<Button onclick={() => (formOpen = false)}>{t('common.cancel')}</Button>
		<Button type="submit" variant="primary" loading={saving}>{t('common.save')}</Button>
	{/snippet}
</Dialog>

<ConfirmDialog
	bind:open={deleteOpen}
	title={t('plans.deleteTitle')}
	message={t('plans.deleteBody', { name: deleteTarget?.name ?? '' })}
	confirmLabel={t('plans.delete')}
	tone="danger"
	loading={deleting}
	onConfirm={confirmDelete}
/>

<style>
	.plan {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: var(--rh-space-1);
	}

	.plan__name {
		font-weight: var(--rh-font-weight-semibold);
		white-space: nowrap;
	}

	.plan__host {
		white-space: nowrap;
	}

	.plan__meta {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.plan__cron {
		font-family: var(--rh-font-mono);
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

	.runs {
		margin: 0;
		padding-left: var(--rh-space-4);
	}

	.presets {
		display: flex;
		flex-wrap: wrap;
		gap: var(--rh-space-2);
	}

	.preview__hint {
		color: var(--rh-color-text-muted);
	}
</style>
