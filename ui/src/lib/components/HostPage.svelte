<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { resolve } from '$app/paths';
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import type {
		Agent,
		Alert as AlertItem,
		ApiProblem,
		HostParameterRow,
		Job,
		ParameterValueDef,
		Plan,
		Secret,
		User
	} from '#lib/api/index.ts';
	import { eventReader } from '#lib/events.ts';
	import HostActions from './HostActions.svelte';
	import ProfileSection from './ProfileSection.svelte';
	import SecretDialog from './SecretDialog.svelte';
	import { RecordPage, SummaryPanel } from './layout/index.ts';
	import {
		Alert,
		Badge,
		Button,
		CopyField,
		DangerZone,
		DataTable,
		Icon,
		ListView,
		Select,
		StatusPill,
		TextField
	} from './ui/index.ts';
	import type {
		DangerAction,
		KeyValueItem,
		SelectOption,
		StatusTone,
		TableColumn
	} from './ui/index.ts';
	import { isTimed, jobDuration, jobMeta, jobTitle } from '#lib/job-item.ts';
	import { iconOf, jobState, pillOf } from '#lib/vocabulary.ts';
	import { formatTime, relativeTime } from '#lib/format.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import type { SummaryState } from '#lib/profile-form.ts';
	import { osFamily } from '#lib/host-picker.ts';
	import { enableCommand } from '#lib/scripts.ts';
	import { kindLabel, parameterCount, plansRefusal } from '#lib/parameters.ts';
	import type { PlansRefusal } from '#lib/parameters.ts';
	import { plural } from '#lib/list-format.ts';
	import { onEvent } from '#lib/stores/events.svelte.ts';
	import { rememberSections } from '#lib/section-state.svelte.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';
	import { keepUnchanged } from '#lib/unchanged.ts';

	let { hostId }: { hostId: string } = $props();

	type SectionKey = 'general' | 'link' | 'restore' | 'parameters' | 'scripts' | 'jobs' | 'danger';
	const SECTIONS: SectionKey[] = [
		'general',
		'link',
		'restore',
		'parameters',
		'scripts',
		'jobs',
		'danger'
	];
	const RECENT_JOBS = 5;
	// An agent rotates at two thirds of the lifetime, so a certificate that is close to its end
	// means that the agent could not rotate (FR-ID-03).
	const CERT_WARNING_MS = 3 * 24 * 60 * 60 * 1000;

	let loading = $state(true);
	let loadError = $state<string | null>(null);
	let agent = $state<Agent | null>(null);
	let users = $state<User[]>([]);
	let jobs = $state<Job[]>([]);
	let plans = $state<Plan[]>([]);
	let openAlerts = $state<AlertItem[]>([]);
	let secrets = $state<Secret[]>([]);
	let parameterRows = $state<HostParameterRow[]>([]);
	/** The value that the person types, by parameter name. A secret holds the identifier of a secret. */
	let parameterValues = $state<Record<string, { text: string; secretId: string }>>({});
	/** The values that the person removed; the request leaves them out and the host forgets them. */
	let removedParameters = $state<string[]>([]);
	let parameterBaseline = $state('');
	let parameterSaving = $state(false);
	let parameterProblem = $state<ApiProblem | null>(null);
	let parameterError = $state<string | null>(null);
	let secretOpen = $state(false);
	let secretParameter = $state<string | null>(null);
	let hostActions: HostActions;
	const open = rememberSections(
		'host',
		SECTIONS,
		Object.fromEntries(SECTIONS.map((key) => [key, true]))
	);

	const jobsHref = $derived(`${resolve('/(app)/jobs')}?agent_id=${encodeURIComponent(hostId)}`);
	const restoreHref = $derived(`${resolve('/(app)/restore')}?target=${encodeURIComponent(hostId)}`);
	const canWrite = $derived(getSession()?.role === 'admin');
	// An Operator sets the text values; only an Admin changes a secret value (ADR-0047).
	const canSetText = $derived(['admin', 'operator'].includes(getSession()?.role ?? ''));
	const canSetSecret = $derived(getSession()?.role === 'admin');
	const active = $derived(agent?.status === 'active');
	const olderDuplicate = $derived(
		agent?.possible_duplicate_of?.is_older && active ? agent.possible_duplicate_of : null
	);

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	async function read(): Promise<void> {
		const [agentResult, userList, jobPage, planList, alertPage, parameterResult, secretList] =
			await Promise.allSettled([
				service.getAgent(hostId),
				// Only an Admin lists the users; the others see no name of the revoking user.
				canWrite ? service.listUsers() : Promise.resolve([] as User[]),
				service.listJobs({ agent_id: hostId, limit: RECENT_JOBS }),
				service.listPlans(),
				service.listAlerts({ status: 'open', limit: 200 }),
				service.getAgentParameters(hostId),
				// The secret picker lists the credentials that a parameter may reference.
				canSetText ? service.listSecrets() : Promise.resolve([] as Secret[])
			]);
		if (agentResult.status === 'rejected') throw agentResult.reason;
		agent = agentResult.value;
		users = userList.status === 'fulfilled' ? userList.value : [];
		jobs = keepUnchanged(
			jobs,
			jobPage.status === 'fulfilled' ? jobPage.value.items : [],
			(job) => job.id,
			isTimed
		);
		plans =
			planList.status === 'fulfilled' ? planList.value.filter((p) => p.agent_id === hostId) : [];
		openAlerts =
			alertPage.status === 'fulfilled'
				? alertPage.value.items.filter((item) => item.agent_id === hostId)
				: [];
		secrets = secretList.status === 'fulfilled' ? secretList.value : [];
		if (parameterResult.status === 'fulfilled') applyParameters(parameterResult.value.parameters);
	}

	async function load(): Promise<void> {
		loading = true;
		loadError = null;
		try {
			await read();
		} catch (problem) {
			loadError = messageOf(problem);
		} finally {
			loading = false;
			reader.loaded();
		}
		await tick();
		scrollToHash();
	}

	async function refresh(): Promise<void> {
		try {
			await read();
		} catch {
			// The next event reads again; the visible state stays.
		}
	}

	/* ------------------------------------------------------- parameter values */

	function draftOf(rows: HostParameterRow[]): Record<string, { text: string; secretId: string }> {
		return Object.fromEntries(
			rows.map((row) => [
				row.name,
				{
					text: row.value?.kind === 'text' ? (row.value.value ?? '') : '',
					secretId: row.value?.kind === 'secret' ? (row.value.secret_id ?? '') : ''
				}
			])
		);
	}

	// A refresh keeps the values that the person typed, so an event does not undo an edit.
	function applyParameters(rows: HostParameterRow[]): void {
		parameterRows = rows;
		// The first read fills the draft; a later refresh keeps the values that the person typed.
		if (parameterBaseline !== '' && parameterDirty) return;
		parameterValues = draftOf(rows);
		removedParameters = [];
		parameterBaseline = JSON.stringify([parameterValues, removedParameters]);
	}

	function cancelParameters(): void {
		parameterValues = draftOf(parameterRows);
		removedParameters = [];
		parameterBaseline = JSON.stringify([parameterValues, removedParameters]);
	}

	const parameterDirty = $derived(
		JSON.stringify([parameterValues, removedParameters]) !== parameterBaseline
	);
	const shownParameters = $derived(
		parameterRows.filter((row) => !removedParameters.includes(row.name))
	);

	function valueOf(row: HostParameterRow): { text: string; secretId: string } {
		return parameterValues[row.name] ?? { text: '', secretId: '' };
	}

	// A text parameter without a value falls back to the default of the profile (FR-PF-10).
	function hasValue(row: HostParameterRow): boolean {
		if (row.kind === 'secret') return valueOf(row).secretId !== '';
		return valueOf(row).text.trim() !== '' || Boolean(row.default_value);
	}

	const missingValues = $derived(shownParameters.filter((row) => row.required && !hasValue(row)));
	const secretOptions = $derived<SelectOption[]>([
		{ value: '', label: t('scripts.parameter.selectSecret') },
		...secrets.map((secret) => ({ value: secret.id, label: secret.name }))
	]);

	function setText(row: HostParameterRow, text: string): void {
		parameterValues[row.name] = { ...valueOf(row), text };
	}

	function setSecret(row: HostParameterRow, secretId: string): void {
		parameterValues[row.name] = { ...valueOf(row), secretId };
	}

	/** The value that a Viewer reads: a mask, or the fallback of the profile without the text. */
	function viewerText(row: HostParameterRow): string {
		if (row.value) return '••••••';
		if (row.hidden || row.default_value) return t('hp.param.fromDefaultOnly');
		return '';
	}

	function removeValue(row: HostParameterRow): void {
		removedParameters = [...removedParameters, row.name];
	}

	function openSecret(name: string): void {
		secretParameter = name;
		secretOpen = true;
	}

	function secretCreated(secret: Secret): void {
		secrets = [...secrets, secret];
		if (secretParameter) {
			parameterValues[secretParameter] = {
				...valueOf({ name: secretParameter } as HostParameterRow),
				secretId: secret.id
			};
		}
		pushToast('success', t('scripts.secret.created'));
	}

	async function saveParameters(): Promise<void> {
		parameterSaving = true;
		parameterProblem = null;
		parameterError = null;
		try {
			const body: ParameterValueDef[] = [];
			for (const row of shownParameters) {
				const value = valueOf(row);
				if (row.kind === 'secret') {
					if (value.secretId !== '') {
						body.push({ name: row.name, kind: 'secret', secret_id: value.secretId });
					}
				} else if (value.text.trim() !== '') {
					body.push({ name: row.name, kind: 'text', value: value.text });
				}
			}
			const response = await service.replaceAgentParameters(hostId, body);
			applyParameters(response.parameters);
			pushToast('success', t('hp.param.saved'));
		} catch (problem) {
			if (problem && typeof problem === 'object' && 'problem' in problem) {
				parameterProblem = (problem as { problem: ApiProblem }).problem;
			} else {
				parameterError = messageOf(problem);
			}
		} finally {
			parameterSaving = false;
		}
	}

	const refusedValues = $derived<PlansRefusal | null>(plansRefusal(parameterProblem));

	function parameterFieldError(row: HostParameterRow): string | undefined {
		const index = shownParameters.findIndex((item) => item.name === row.name);
		return (
			fieldError(parameterProblem, `parameters[${index}].value`) ??
			fieldError(parameterProblem, `parameters[${index}].secret_id`)
		);
	}

	function scrollToHash(): void {
		const id = window.location.hash.slice(1);
		if (id) document.getElementById(id)?.scrollIntoView({ block: 'start' });
	}

	const reader = eventReader(() => loading, refresh, 300);
	onMount(() => {
		const off = onEvent((event) => {
			if (
				event.type === 'resync' ||
				event.type === 'host.online' ||
				event.type === 'host.offline' ||
				event.type === 'job.status' ||
				event.type === 'alert.changed'
			) {
				reader.notify();
			}
		});
		void load();
		return off;
	});

	function isWaiting(item: Agent): boolean {
		return item.status === 'active' && !item.first_connected_at;
	}

	const connection = $derived.by<{ label: string; tone: StatusTone } | null>(() => {
		if (!agent) return null;
		if (agent.status === 'revoked') return { label: t('hosts.status.revoked'), tone: 'neutral' };
		if (isWaiting(agent)) return { label: t('hosts.status.waiting'), tone: 'info' };
		return agent.online
			? { label: t('hp.state.online'), tone: 'success' }
			: { label: t('hp.state.offline'), tone: 'neutral' };
	});

	const certLeft = $derived(
		agent ? new Date(agent.certificate_expires_at).getTime() - Date.now() : 0
	);
	const certExpired = $derived(certLeft <= 0);
	const certWarn = $derived(agent?.status === 'active' && certLeft < CERT_WARNING_MS);

	const revokedBy = $derived(
		agent?.revoked_by_user_id
			? users.find((user) => user.id === agent!.revoked_by_user_id)?.display_name
			: undefined
	);

	const hasRoots = $derived((agent?.restore_roots.length ?? 0) > 0);

	function scriptsLabel(item: Agent): string {
		return t(`hosts.serverScripts.${item.server_scripts}` as MessageKey);
	}

	const parameterSummary = $derived.by<{ text: string; state: SummaryState }>(() => {
		if (parameterRows.length === 0) return { text: t('hp.param.none'), state: 'off' };
		if (missingValues.length > 0) {
			return { text: plural('hp.param.missing', missingValues.length), state: 'warn' };
		}
		return { text: parameterCount(parameterRows.length), state: 'on' };
	});

	const summaries = $derived.by<Record<SectionKey, { text: string; state: SummaryState }>>(() => {
		const item = agent;
		if (!item) {
			const blank = { text: '', state: 'off' as SummaryState };
			return {
				general: blank,
				link: blank,
				restore: blank,
				parameters: blank,
				scripts: blank,
				jobs: blank,
				danger: blank
			};
		}
		return {
			general: {
				text: t('hp.summary.general', {
					os: osFamily(item.os),
					arch: item.arch,
					version: item.agent_version
				}),
				state: 'on'
			},
			link: {
				text: t(certExpired ? 'hosts.certExpired' : 'hosts.certExpires', {
					time: relativeTime(item.certificate_expires_at)
				}),
				state: certExpired || certWarn || item.superseded_certificate_used_at ? 'warn' : 'on'
			},
			parameters: parameterSummary,
			restore: hasRoots
				? {
						text:
							item.restore_roots.length === 1
								? t('hp.summary.root')
								: t('hp.summary.roots', { count: item.restore_roots.length }),
						state: 'on'
					}
				: { text: t('hp.summary.noRoots'), state: 'warn' },
			scripts: {
				text: scriptsLabel(item),
				state: item.server_scripts === 'enabled' ? 'on' : 'off'
			},
			jobs: {
				text: t('hp.summary.jobs', { count: jobs.length }),
				state: jobs.length > 0 ? 'on' : 'off'
			},
			danger: { text: t('hp.summary.danger'), state: 'off' }
		};
	});

	// The record keeps each destructive action here and nowhere else (UI principles 1.7).
	const dangerActions = $derived.by<DangerAction[]>(() => {
		const item = agent;
		if (!item) return [];
		return [
			{
				key: 'reregister',
				text: t('hp.danger.reregister'),
				label: t('hosts.reregister'),
				icon: 'key',
				onclick: () => hostActions.openReregister(item)
			},
			...(olderDuplicate
				? [
						{
							key: 'replace',
							text: t('hp.danger.replace', { old: olderDuplicate.hostname }),
							label: t('hosts.replaceOld'),
							icon: 'host' as const,
							tone: 'danger' as const,
							onclick: () => hostActions.openReplace(item)
						}
					]
				: []),
			{
				key: 'revoke',
				text: t('hp.danger.revoke'),
				label: t('hosts.revokeAgent'),
				icon: 'octagon',
				tone: 'danger',
				onclick: () => hostActions.openRevoke(item)
			}
		];
	});

	const stateItems = $derived.by<KeyValueItem[]>(() => {
		const item = agent;
		if (!item) return [];
		return [
			{
				key: 'connection',
				label: t('hp.state.connection'),
				value: connection?.label ?? '',
				render: connectionValue
			},
			...(item.online
				? []
				: [
						{
							key: 'seen',
							label: t('hosts.lastSeen'),
							value: item.last_seen_at ? relativeTime(item.last_seen_at) : t('common.never')
						}
					]),
			{
				key: 'alerts',
				label: t('hp.state.alerts'),
				value: openAlerts.length > 0 ? String(openAlerts.length) : t('hp.state.alertsNone'),
				render: openAlerts.length > 0 ? alertsValue : undefined
			},
			{ key: 'certificate', label: t('hosts.certificate'), value: summaries.link.text },
			{ key: 'scripts', label: t('hosts.serverScripts'), value: summaries.scripts.text }
		];
	});
</script>

{#snippet time(value: string | null | undefined)}
	{#if value}
		<time datetime={value} title={formatTime(value)}>{relativeTime(value)}</time>
	{:else}
		{t('common.never')}
	{/if}
{/snippet}

{#snippet connectionValue()}
	{#if connection}
		<StatusPill tone={connection.tone}>{connection.label}</StatusPill>
	{/if}
{/snippet}

{#snippet alertsValue()}
	{openAlerts.length}
	<a class="link" href={resolve('/(app)/alerts')}>{t('hp.state.alertsLink')}</a>
{/snippet}

{#snippet parameterNameCell(row: HostParameterRow)}
	<span class="mono">{row.name}</span>
	{#if row.required}
		<span class="param-note">· {t('parameters.requiredShort')}</span>
	{/if}
{/snippet}

{#snippet parameterKindCell(row: HostParameterRow)}
	<span class="kind">
		{#if row.kind === 'secret'}<Icon name="lock" size="xs" />{/if}
		{kindLabel(row.kind)}
	</span>
{/snippet}

{#snippet parameterValueCell(row: HostParameterRow)}
	{#if row.kind === 'secret'}
		{#if canSetSecret}
			<div class="value-row">
				<Select
					value={valueOf(row).secretId}
					options={secretOptions}
					aria-label={t('hp.param.valueOf', { name: row.name })}
					onchange={(event) => setSecret(row, event.currentTarget.value)}
					error={parameterFieldError(row)}
				/>
				<Button variant="ghost" size="sm" icon="plus" onclick={() => openSecret(row.name)}>
					{t('scripts.secret.new')}
				</Button>
			</div>
		{:else}
			<TextField
				disabled
				value={row.value?.secret_name ?? ''}
				aria-label={t('hp.param.valueOf', { name: row.name })}
				title={t('hp.param.secretLocked')}
			/>
		{/if}
	{:else if !canSetText}
		<TextField
			disabled
			value={viewerText(row)}
			aria-label={t('hp.param.valueOf', { name: row.name })}
		/>
	{:else}
		<TextField
			value={valueOf(row).text}
			oninput={(event) => setText(row, event.currentTarget.value)}
			mono
			aria-label={t('hp.param.valueOf', { name: row.name })}
			placeholder={row.default_value
				? t('hp.param.fromDefault', { value: row.default_value })
				: undefined}
			error={parameterFieldError(row)}
		/>
	{/if}
{/snippet}

{#snippet parameterNeededCell(row: HostParameterRow)}
	{#if row.unused}
		<span class="muted">{t('hp.param.unused')}</span>
	{:else}
		<span class="nowrap">{row.needed_by.join(', ')}</span>
	{/if}
{/snippet}

<RecordPage
	title={agent?.hostname ?? ''}
	description={t('hosts.description')}
	{loading}
	error={loadError ?? (!loading && !agent ? t('table.error') : null)}
>
	{#snippet status()}
		{#if connection}
			<StatusPill tone={connection.tone}>{connection.label}</StatusPill>
		{/if}
	{/snippet}
	{#snippet actions()}
		{#if canWrite && active && agent}
			<Button size="sm" icon="tag" onclick={() => hostActions.openRename(agent!)}>
				{t('hosts.rename')}
			</Button>
		{/if}
	{/snippet}
	{#snippet banners()}
		{#if agent?.status === 'revoked'}
			<Alert tone="attention" icon="octagon" title={t('hp.revokedNote')}>
				{#if revokedBy}
					{t('hp.revokedBy', { name: revokedBy })}
				{/if}
			</Alert>
		{/if}
		{#if olderDuplicate}
			<div class="banner">
				<Alert
					tone="attention"
					title={t('hp.duplicateBanner', {
						old: olderDuplicate.hostname,
						oldSeen: olderDuplicate.last_seen_at
							? relativeTime(olderDuplicate.last_seen_at)
							: t('common.never')
					})}
				>
					{#snippet actions()}
						{#if canWrite}
							<Button size="sm" onclick={() => hostActions.openReplace(agent!)}>
								{t('hosts.replaceOld')}
							</Button>
						{/if}
					{/snippet}
				</Alert>
			</div>
		{/if}
	{/snippet}
	{#snippet side()}
		<SummaryPanel title={t('hp.state.title')} items={stateItems} />
	{/snippet}

	{#if agent}
		<ProfileSection
			id="general"
			title={t('hp.section.general')}
			summary={summaries.general.text}
			state={summaries.general.state}
			bind:open={open.general}
		>
			<dl class="facts">
				<dt>{t('hosts.os')}</dt>
				<dd>{osFamily(agent.os)}</dd>
				<dt>{t('hosts.arch')}</dt>
				<dd>{agent.arch}</dd>
				<dt>{t('hosts.version')}</dt>
				<dd>
					<span class="mono">{agent.agent_version}</span>
					{#if agent.version_status === 'update_available' && active}
						<Badge tone="warning">{t('hosts.versionUpdate')}</Badge>
					{:else if agent.version_status === 'unsupported'}
						<Badge tone="danger">{t('hosts.versionUnsupported')}</Badge>
					{/if}
				</dd>
				<dt>{t('hp.f.restic')}</dt>
				<dd class="mono">{agent.restic_version}</dd>
				<dt>{t('hosts.labels')}</dt>
				<dd>
					{#if agent.labels.length > 0}
						<span class="chips">
							{#each agent.labels as label (label)}
								<Badge tone="neutral">{label}</Badge>
							{/each}
						</span>
					{:else}
						<span class="muted">{t('common.none')}</span>
					{/if}
				</dd>
				<dt>{t('hosts.lastSeen')}</dt>
				<dd>{@render time(agent.last_seen_at)}</dd>
				<dt>{t('hp.f.enrolled')}</dt>
				<dd>{@render time(agent.created_at)}</dd>
			</dl>
		</ProfileSection>

		<ProfileSection
			id="link"
			title={t('hp.section.link')}
			summary={summaries.link.text}
			state={summaries.link.state}
			bind:open={open.link}
		>
			{#if agent.superseded_certificate_used_at}
				<Alert
					variant="inline"
					tone="danger"
					title={t('hp.cert.superseded', {
						time: relativeTime(agent.superseded_certificate_used_at)
					})}
				/>
			{/if}
			<dl class="facts">
				<dt>{t('hp.f.protocol')}</dt>
				<dd class="mono">{agent.protocol_version ?? t('common.none')}</dd>
				<dt>{t('hp.f.controlPlane')}</dt>
				<dd class="mono">{agent.control_plane_version}</dd>
				<dt>{t('hp.f.firstConnected')}</dt>
				<dd>{@render time(agent.first_connected_at)}</dd>
				<dt>{t('hp.f.certExpires')}</dt>
				<dd class:warn={certWarn || certExpired}>
					<time
						datetime={agent.certificate_expires_at}
						title={formatTime(agent.certificate_expires_at)}
					>
						{t(certExpired ? 'hosts.certExpired' : 'hosts.certExpires', {
							time: relativeTime(agent.certificate_expires_at)
						})}
					</time>
				</dd>
				<dt>{t('hp.f.rotation')}</dt>
				<dd>
					{#if agent.last_certificate_rotation_at}
						{@render time(agent.last_certificate_rotation_at)}
					{:else}
						<span class="muted">{t('hosts.certNotRotated')}</span>
					{/if}
				</dd>
			</dl>
			<CopyField value={agent.certificate_serial} label={t('hp.f.serial')} />
		</ProfileSection>

		<ProfileSection
			id="restore"
			title={t('hp.section.restore')}
			summary={summaries.restore.text}
			state={summaries.restore.state}
			bind:open={open.restore}
		>
			<p class="lead">
				{t(agent.restore_roots.length === 1 ? 'hp.restore.ruleOne' : 'hp.restore.rule')}
			</p>
			{#if hasRoots}
				<div class="roots">
					<span class="label"
						>{t(agent.restore_roots.length === 1 ? 'hp.restore.root' : 'hp.restore.roots')}</span
					>
					{#each agent.restore_roots as root (root)}
						<CopyField value={root} />
					{/each}
				</div>
			{:else}
				<Alert variant="inline" tone="attention" title={t('hp.restore.none')} />
				<p class="lead">{t('hp.restore.noneHint')}</p>
			{/if}
			{#if active}
				<!-- eslint-disable svelte/no-navigation-without-resolve -- resolved path with a query -->
				<a class="link" href={restoreHref}>
					{t('hp.restore.link')}
				</a>
				<!-- eslint-enable svelte/no-navigation-without-resolve -->
			{/if}
		</ProfileSection>

		<div id="parameters" class="anchor">
			<ProfileSection
				id="parameters"
				title={t('hp.section.parameters')}
				summary={summaries.parameters.text}
				state={summaries.parameters.state}
				bind:open={open.parameters}
			>
				<p class="lead">{t('hp.param.hint')}</p>
				{#if !canSetText}
					<Alert variant="inline" tone="info" icon="eye-off" title={t('hp.param.viewer')} />
				{:else if !canSetSecret}
					<Alert variant="inline" tone="info" icon="lock" title={t('hp.param.secretLocked')} />
				{/if}
				{#if refusedValues}
					<Alert
						tone="danger"
						title={t('hp.param.refusedTitle', { names: refusedValues.names.join(', ') })}
					>
						<ul class="blockers">
							{#each refusedValues.plans.links as link (link.href + link.label)}
								<li>
									<span class="blockers__kind">{link.kind}</span>
									<!-- The refusal resolves the address of the plan with `resolve()`. -->
									<!-- eslint-disable-next-line svelte/no-navigation-without-resolve -->
									<a href={link.href}>{link.label}</a>
								</li>
							{/each}
							{#if refusedValues.plans.more > 0}
								<li class="blockers__more">
									{t('deleteBlocked.more', { n: refusedValues.plans.more })}
								</li>
							{/if}
						</ul>
					</Alert>
				{/if}
				{#if parameterError}
					<Alert tone="danger" title={parameterError} />
				{/if}
				{#if parameterRows.length === 0}
					<p class="muted">{t('hp.param.none')}</p>
				{:else}
					<DataTable
						paginated={false}
						rowKey={(row) => row.name}
						rows={shownParameters}
						columns={[
							{
								key: 'name',
								header: t('parameters.name'),
								sortable: false,
								nowrap: true,
								render: parameterNameCell
							},
							{
								key: 'kind',
								header: t('parameters.kind'),
								sortable: false,
								nowrap: true,
								render: parameterKindCell
							},
							{
								key: 'value',
								header: t('hp.param.value'),
								sortable: false,
								render: parameterValueCell
							},
							{
								key: 'needed',
								header: t('hp.param.neededBy'),
								sortable: false,
								render: parameterNeededCell
							}
						] satisfies TableColumn<HostParameterRow>[]}
						actions={(row) =>
							canSetText && row.unused
								? [
										{
											id: 'remove',
											label: t('hp.param.remove', { name: row.name }),
											hint: t('hp.param.remove', { name: row.name }),
											icon: 'trash',
											danger: true,
											onSelect: () => removeValue(row)
										}
									]
								: []}
					/>
					{#if canSetText}
						<div class="param-actions">
							{#if parameterDirty}
								<span class="muted">{t('hp.param.dirty')}</span>
							{/if}
							<span class="spacer"></span>
							{#if parameterDirty}
								<Button onclick={cancelParameters}>{t('common.cancel')}</Button>
							{/if}
							<Button
								variant="primary"
								loading={parameterSaving}
								disabled={!parameterDirty}
								title={parameterDirty ? undefined : t('record.save.clean')}
								onclick={() => void saveParameters()}
							>
								{t('hp.param.save')}
							</Button>
						</div>
					{/if}
				{/if}
			</ProfileSection>
		</div>

		<div id="scripts" class="anchor">
			<ProfileSection
				id="scripts"
				title={t('hp.section.scripts')}
				summary={summaries.scripts.text}
				state={summaries.scripts.state}
				bind:open={open.scripts}
			>
				<dl class="facts">
					<dt>{t('hosts.serverScripts')}</dt>
					<dd>{scriptsLabel(agent)}</dd>
					<dt>{t('hosts.serverScripts.interpreters')}</dt>
					<dd>
						{agent.script_interpreters.length > 0
							? agent.script_interpreters.join(', ')
							: t('common.none')}
					</dd>
				</dl>
				{#if agent.server_scripts === 'enabled'}
					<Alert
						variant="inline"
						tone="attention"
						icon="shield"
						title={t('hosts.serverScripts.risk')}
					/>
				{:else}
					<p class="lead">{t('hosts.serverScripts.enableHint')}</p>
					<CopyField value={enableCommand(agent.os)} />
				{/if}
			</ProfileSection>
		</div>

		<ProfileSection
			id="jobs"
			title={t('hp.section.jobs')}
			summary={summaries.jobs.text}
			state={summaries.jobs.state}
			bind:open={open.jobs}
		>
			<div class="list-block">
				<span class="label">{t('hp.jobs.recent')}</span>
				{#if jobs.length === 0}
					<p class="muted">{t('hp.jobs.none')}</p>
				{:else}
					<ListView
						paginated={false}
						label={t('hp.jobs.recent')}
						items={jobs}
						itemKey={(job) => job.id}
						icon={(job) => iconOf(jobState(job))}
						pill={(job) => pillOf(jobState(job))}
						title={jobTitle}
						href={(job) => resolve('/(app)/jobs/[id]', { id: job.id })}
						meta={(job) => jobMeta(job, { host: false })}
						aside={jobDuration}
					/>
					<!-- eslint-disable svelte/no-navigation-without-resolve -- resolved path with a query -->
					<a class="link" href={jobsHref}>
						{t('hp.jobs.all')}
					</a>
					<!-- eslint-enable svelte/no-navigation-without-resolve -->
				{/if}
			</div>
			<div class="list-block">
				<span class="label">{t('hp.plans.title')}</span>
				{#if plans.length === 0}
					<p class="muted">{t('hp.plans.none')}</p>
				{:else}
					<ul class="rows">
						{#each plans as plan (plan.id)}
							<li>
								<span class="rows__main">{plan.name}</span>
								<span class="muted nowrap">{plan.cron}</span>
							</li>
						{/each}
					</ul>
					<a class="link" href={resolve('/(app)/plans')}>{t('hp.plans.all')}</a>
				{/if}
			</div>
		</ProfileSection>

		{#if canWrite && active}
			<ProfileSection
				id="danger"
				title={t('danger.title')}
				summary={summaries.danger.text}
				state={summaries.danger.state}
				bind:open={open.danger}
			>
				<DangerZone actions={dangerActions} />
			</ProfileSection>
		{/if}
	{/if}
</RecordPage>

<HostActions bind:this={hostActions} onchanged={read} />

<SecretDialog bind:open={secretOpen} oncreated={secretCreated} />

<style>
	.banner {
		margin-bottom: var(--rh-space-4);
	}

	.anchor {
		scroll-margin-top: calc(var(--rh-layout-topbar-height) + var(--rh-space-4));
	}

	.facts {
		display: grid;
		grid-template-columns: minmax(140px, max-content) minmax(0, 1fr);
		gap: var(--rh-space-2) var(--rh-space-4);
		margin: 0;
	}

	.facts dt {
		color: var(--rh-color-text-muted);
	}

	.facts dd {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--rh-space-2);
		margin: 0;
		min-width: 0;
		overflow-wrap: anywhere;
	}

	.warn {
		color: var(--rh-color-danger-text);
		font-weight: var(--rh-font-weight-semibold);
	}

	.mono {
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-sm);
	}

	.muted {
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.nowrap {
		white-space: nowrap;
	}

	.lead {
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.label {
		font-weight: var(--rh-font-weight-semibold);
	}

	.chips {
		display: flex;
		flex-wrap: wrap;
		gap: var(--rh-space-1);
	}

	.roots,
	.list-block {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-2);
	}

	.link {
		align-self: flex-start;
		font-size: var(--rh-font-size-sm);
	}

	.rows {
		display: flex;
		flex-direction: column;
		margin: 0;
		padding: 0;
		list-style: none;
	}

	.rows li {
		display: flex;
		align-items: center;
		gap: var(--rh-space-3);
		padding: var(--rh-space-2) 0;
		border-top: var(--rh-border-width) solid var(--rh-color-border-muted);
	}

	.rows li:first-child {
		border-top: 0;
	}

	.kind {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-1);
		color: var(--rh-color-text-muted);
	}

	.param-note {
		margin-inline-start: var(--rh-space-1);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-xs);
	}

	.value-row {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.value-row :global(.rh-field) {
		flex: 1;
		min-width: 0;
	}

	.param-actions {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.param-actions .spacer {
		flex: 1;
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

	.blockers__more {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.rows__main {
		flex: 1;
		min-width: 0;
		overflow-wrap: anywhere;
	}

	@media (max-width: 720px) {
		.banner :global(.alert) {
			flex-wrap: wrap;
		}

		.banner :global(.alert__body) {
			flex-basis: calc(100% - 44px);
		}

		.banner :global(.alert__actions) {
			margin-left: 36px;
		}

		.facts {
			grid-template-columns: minmax(0, 1fr);
			gap: 0;
		}

		.facts dd {
			margin-bottom: var(--rh-space-2);
		}
	}
</style>
