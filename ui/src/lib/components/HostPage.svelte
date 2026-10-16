<script lang="ts">
	import { onDestroy, onMount, tick } from 'svelte';
	import { resolve } from '$app/paths';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { Agent, Alert as AlertItem, Job, Plan, User } from '#lib/api/index.ts';
	import { eventReader } from '#lib/events.ts';
	import HostActions from './HostActions.svelte';
	import ProfileSection from './ProfileSection.svelte';
	import {
		Alert,
		Badge,
		Button,
		CopyField,
		DropdownMenu,
		Icon,
		PageHeader,
		Skeleton,
		StatusPill
	} from './ui/index.ts';
	import type { MenuItem, StatusTone } from './ui/index.ts';
	import { formatTime, relativeTime } from '#lib/format.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import type { SummaryState } from '#lib/profile-form.ts';
	import { enableCommand } from '#lib/scripts.ts';
	import { setPageTitle } from '#lib/stores/page-title.svelte.ts';
	import { onEvent } from '#lib/stores/events.svelte.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';

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
	let open = $state<Record<string, boolean>>(
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
		setPageTitle(agent.hostname);
		users = userList.status === 'fulfilled' ? userList.value : [];
		jobs = jobPage.status === 'fulfilled' ? jobPage.value.items : [];
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
	onDestroy(() => setPageTitle(null));

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

	function jobLabel(job: Job): string {
		if (job.status === 'failed' && job.result_code === 'cancelled')
			return t('jobs.status.cancelled');
		if (job.status === 'succeeded' && job.result_code === 'succeeded_with_warnings') {
			return t('jobs.status.succeeded_with_warnings');
		}
		return t(`jobs.status.${job.status}` as MessageKey);
	}

	function jobFullLabel(job: Job): string | undefined {
		return job.status === 'succeeded' && job.result_code === 'succeeded_with_warnings'
			? t('jobs.statusFull.succeeded_with_warnings')
			: undefined;
	}

	function jobTone(job: Job): StatusTone {
		if (job.status === 'failed' && job.result_code === 'cancelled') return 'cancelled';
		if (job.status === 'succeeded' && job.result_code === 'succeeded_with_warnings') {
			return 'warning';
		}
		switch (job.status) {
			case 'queued':
				return 'queued';
			case 'sent':
				return 'info';
			case 'running':
				return 'running';
			case 'succeeded':
				return 'success';
			case 'expired':
				return 'warning';
			default:
				return 'failed';
		}
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
					os: item.os,
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
				? { text: t('hp.summary.roots', { count: item.restore_roots.length }), state: 'on' }
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

	const menuItems = $derived.by<MenuItem[]>(() => {
		const item = agent;
		if (!item) return [];
		return [
			{
				label: t('hosts.reregister'),
				icon: 'key',
				onSelect: () => hostActions.openReregister(item)
			},
			{
				label: t('hosts.revokeAgent'),
				icon: 'octagon',
				tone: 'danger',
				separatorBefore: true,
				onSelect: () => hostActions.openRevoke(item)
			}
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

<svelte:head>
	<title>{agent?.hostname ?? t('hosts.title')} · {t('app.name')}</title>
</svelte:head>

<a class="back" href={resolve('/(app)/hosts')}>
	<Icon name="arrow-left" size="xs" />
	{t('hp.back')}
</a>

{#if loading}
	<Skeleton />
{:else if loadError || !agent}
	<Alert tone="danger" title={loadError ?? t('table.error')} />
	<p><a href={resolve('/(app)/hosts')}>{t('hp.back')}</a></p>
{:else}
	<PageHeader title={agent.hostname} description={t('hp.description')}>
		{#if canWrite && active}
			<Button size="sm" icon="tag" onclick={() => hostActions.openRename(agent!)}>
				{t('hosts.rename')}
			</Button>
			<DropdownMenu
				icon="more"
				align="end"
				label={t('hosts.moreActions', { name: agent.hostname })}
				items={menuItems}
			/>
		{/if}
	</PageHeader>
	{#if connection}
		<div class="state-row">
			<StatusPill tone={connection.tone}>{connection.label}</StatusPill>
		</div>
	{/if}

	{#if agent.status === 'revoked'}
		<div class="banner">
			<Alert tone="attention" icon="octagon" title={t('hp.revokedNote')}>
				{#if revokedBy}
					{t('hp.revokedBy', { name: revokedBy })}
				{/if}
			</Alert>
		</div>
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

	<div class="layout">
		<div class="main">
			<ProfileSection
				id="general"
				title={t('hp.section.general')}
				summary={summaries.general.text}
				state={summaries.general.state}
				bind:open={open.general}
			>
				<dl class="facts">
					<dt>{t('hosts.os')}</dt>
					<dd>{agent.os}</dd>
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
				<p class="lead">{t('hp.restore.rule')}</p>
				{#if hasRoots}
					<div class="roots">
						<span class="label">{t('hp.restore.roots')}</span>
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
						<ul class="rows">
							{#each jobs as job (job.id)}
								<li>
									<StatusPill tone={jobTone(job)} title={jobFullLabel(job)}
										>{jobLabel(job)}</StatusPill
									>
									<a class="rows__main" href={resolve('/(app)/jobs/[id]', { id: job.id })}>
										{job.plan_name ?? t(`jobs.kind.${job.kind}` as MessageKey)}
									</a>
									<span class="muted nowrap">{@render time(job.created_at)}</span>
								</li>
							{/each}
						</ul>
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
					title={t('hp.section.danger')}
					summary={summaries.danger.text}
					state={summaries.danger.state}
					bind:open={open.danger}
				>
					<ul class="danger">
						<li>
							<p class="lead">{t('hp.danger.reregister')}</p>
							<Button icon="key" onclick={() => hostActions.openReregister(agent!)}>
								{t('hosts.reregister')}
							</Button>
						</li>
						{#if olderDuplicate}
							<li>
								<p class="lead">{t('hp.danger.replace', { old: olderDuplicate.hostname })}</p>
								<Button
									variant="danger"
									icon="host"
									onclick={() => hostActions.openReplace(agent!)}
								>
									{t('hosts.replaceOld')}
								</Button>
							</li>
						{/if}
						<li>
							<p class="lead">{t('hp.danger.revoke')}</p>
							<Button
								variant="danger"
								icon="octagon"
								onclick={() => hostActions.openRevoke(agent!)}
							>
								{t('hosts.revokeAgent')}
							</Button>
						</li>
					</ul>
				</ProfileSection>
			{/if}
		</div>

		<aside class="side">
			<div class="box">
				<div class="box__head">
					<span>{t('hp.state.title')}</span>
				</div>
				<ul class="result">
					<li>
						<span
							class="dot dot--{connection?.tone === 'success' ? 'on' : 'off'}"
							data-state={connection?.tone === 'success' ? 'on' : 'off'}
						></span>
						<div>
							<b>{t('hp.state.connection')}</b>
							<span class="result__text">{connection?.label}</span>
						</div>
					</li>
					{#if !agent.online}
						<li>
							<span class="dot dot--off"></span>
							<div>
								<b>{t('hosts.lastSeen')}</b>
								<span class="result__text">{@render time(agent.last_seen_at)}</span>
							</div>
						</li>
					{/if}
					<li>
						<span class="dot dot--{openAlerts.length > 0 ? 'warn' : 'on'}"></span>
						<div>
							<b>{t('hp.state.alerts')}</b>
							<span class="result__text">
								{openAlerts.length > 0 ? openAlerts.length : t('hp.state.alertsNone')}
							</span>
							{#if openAlerts.length > 0}
								<a class="link" href={resolve('/(app)/alerts')}>{t('hp.state.alertsLink')}</a>
							{/if}
						</div>
					</li>
					<li>
						<span class="dot dot--{summaries.link.state}"></span>
						<div>
							<b>{t('hosts.certificate')}</b>
							<span class="result__text">{summaries.link.text}</span>
						</div>
					</li>
					<li>
						<span class="dot dot--{summaries.scripts.state}"></span>
						<div>
							<b>{t('hosts.serverScripts')}</b>
							<span class="result__text">{summaries.scripts.text}</span>
						</div>
					</li>
				</ul>
			</div>
		</aside>
	</div>
{/if}

<HostActions bind:this={hostActions} onchanged={read} />

<style>
	.back {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-1);
		margin-bottom: var(--rh-space-3);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
		text-decoration: none;
	}

	.back:hover {
		color: var(--rh-color-link);
	}

	.state-row {
		margin: calc(-1 * var(--rh-space-4)) 0 var(--rh-space-4);
	}

	.banner {
		margin-bottom: var(--rh-space-4);
	}

	.layout {
		display: grid;
		grid-template-columns: minmax(0, 1fr) 340px;
		gap: var(--rh-space-4);
		align-items: start;
	}

	.main {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
		min-width: 0;
	}

	.anchor {
		scroll-margin-top: calc(var(--rh-layout-topbar-height) + var(--rh-space-4));
	}

	.side {
		position: sticky;
		top: calc(var(--rh-layout-topbar-height) + var(--rh-space-4));
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
	}

	.box {
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-lg);
		background-color: var(--rh-color-surface);
	}

	.box__head {
		padding: var(--rh-space-2) var(--rh-space-3);
		border-bottom: var(--rh-border-width) solid var(--rh-color-border-muted);
		font-weight: var(--rh-font-weight-semibold);
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
		overflow-wrap: anywhere;
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

	.danger {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-4);
		margin: 0;
		padding: 0;
		list-style: none;
	}

	.danger li {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: var(--rh-space-3);
	}

	.danger .lead {
		flex: 1 1 260px;
	}

	@media (max-width: 1000px) {
		.layout {
			grid-template-columns: minmax(0, 1fr);
		}

		.side {
			position: static;
			order: -1;
		}
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
