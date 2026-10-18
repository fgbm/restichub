<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { resolve } from '$app/paths';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { Agent, Alert as AlertItem, Job, Plan, User } from '#lib/api/index.ts';
	import { eventReader } from '#lib/events.ts';
	import HostActions from './HostActions.svelte';
	import ProfileSection from './ProfileSection.svelte';
	import { RecordPage, SummaryPanel } from './layout/index.ts';
	import { Alert, Badge, Button, CopyField, DangerZone, ListView, StatusPill } from './ui/index.ts';
	import type { DangerAction, KeyValueItem, StatusTone } from './ui/index.ts';
	import { isTimed, jobDuration, jobMeta, jobTitle } from '#lib/job-item.ts';
	import { iconOf, jobState, pillOf } from '#lib/vocabulary.ts';
	import { formatTime, relativeTime } from '#lib/format.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import type { SummaryState } from '#lib/profile-form.ts';
	import { osFamily } from '#lib/host-picker.ts';
	import { enableCommand } from '#lib/scripts.ts';
	import { onEvent } from '#lib/stores/events.svelte.ts';
	import { rememberSections } from '#lib/section-state.svelte.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { keepUnchanged } from '#lib/unchanged.ts';

	let { hostId }: { hostId: string } = $props();

	type SectionKey = 'general' | 'link' | 'restore' | 'scripts' | 'jobs' | 'danger';
	const SECTIONS: SectionKey[] = ['general', 'link', 'restore', 'scripts', 'jobs', 'danger'];
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
	let hostActions: HostActions;
	const open = rememberSections(
		'host',
		SECTIONS,
		Object.fromEntries(SECTIONS.map((key) => [key, true]))
	);

	const jobsHref = $derived(`${resolve('/(app)/jobs')}?agent_id=${encodeURIComponent(hostId)}`);
	const restoreHref = $derived(`${resolve('/(app)/restore')}?target=${encodeURIComponent(hostId)}`);
	const canWrite = $derived(getSession()?.role === 'admin');
	const active = $derived(agent?.status === 'active');
	const olderDuplicate = $derived(
		agent?.possible_duplicate_of?.is_older && active ? agent.possible_duplicate_of : null
	);

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	async function read(): Promise<void> {
		const [agentResult, userList, jobPage, planList, alertPage] = await Promise.allSettled([
			service.getAgent(hostId),
			// Only an Admin lists the users; the others see no name of the revoking user.
			canWrite ? service.listUsers() : Promise.resolve([] as User[]),
			service.listJobs({ agent_id: hostId, limit: RECENT_JOBS }),
			service.listPlans(),
			service.listAlerts({ status: 'open', limit: 200 })
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

	const summaries = $derived.by<Record<SectionKey, { text: string; state: SummaryState }>>(() => {
		const item = agent;
		if (!item) {
			const blank = { text: '', state: 'off' as SummaryState };
			return {
				general: blank,
				link: blank,
				restore: blank,
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
