<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { AlertKind, Dashboard, LockState, RpoState } from '#lib/api/index.ts';
	import {
		Alert,
		Button,
		Card,
		EmptyState,
		Icon,
		PageHeader,
		Panel,
		Skeleton,
		StatusPill
	} from '#lib/components/ui/index.ts';
	import type { IconName, StatusTone } from '#lib/components/ui/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { alertCondition } from '#lib/alert-text.ts';
	import { formatTime, relativeTime } from '#lib/format.ts';
	import { isSingleTenant } from '#lib/stores/instance.svelte.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';

	interface Step {
		title: string;
		body: string;
		done: boolean;
		action?: string;
		onAction?: () => void;
	}

	const session = $derived(getSession());
	// An account with no current organization sees the empty state (ADR-0029).
	const hasTenant = $derived(!!getSession()?.tenant);

	// The repositories and profiles screens arrive later, so their actions stay disabled.
	const canSeeHosts = $derived(['admin', 'operator', 'viewer'].includes(session?.role ?? ''));
	const steps = $derived<Step[]>([
		{
			title: isSingleTenant() ? t('setup.submitSingle') : t('overview.step.admin.title'),
			body: t('overview.step.admin.body', {
				name: session?.user?.display_name || session?.user?.username || ''
			}),
			done: true
		},
		{
			title: t('overview.step.agent.title'),
			body: t('overview.step.agent.body'),
			done: false,
			action: t('overview.step.agent.action'),
			onAction: canSeeHosts ? () => void goto(resolve('/(app)/hosts')) : undefined
		},
		{
			title: t('overview.step.repository.title'),
			body: t('overview.step.repository.body'),
			done: false,
			action: t('overview.step.repository.action')
		},
		{
			title: t('overview.step.profile.title'),
			body: t('overview.step.profile.body'),
			done: false,
			action: t('overview.step.profile.action')
		}
	]);

	const done = $derived(steps.filter((step) => step.done).length);

	const empties: { icon: IconName; title: MessageKey; body: MessageKey }[] = [
		{ icon: 'host', title: 'overview.empty.hosts.title', body: 'overview.empty.hosts.body' },
		{ icon: 'job', title: 'overview.empty.jobs.title', body: 'overview.empty.jobs.body' },
		{
			icon: 'repository',
			title: 'overview.empty.repositories.title',
			body: 'overview.empty.repositories.body'
		}
	];

	const hints: MessageKey[] = [
		'overview.how.schedule',
		'overview.how.data',
		'overview.how.maintenance'
	];

	let dashboard = $state<Dashboard | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);

	// An organization with no host, no plan and no repository still shows the first steps.
	const firstRun = $derived(
		dashboard !== null &&
			dashboard.hosts.total === 0 &&
			dashboard.plans.total === 0 &&
			dashboard.repositories.length === 0
	);

	const RPO_TONES: Record<RpoState, StatusTone> = {
		ok: 'success',
		at_risk: 'warning',
		violated: 'failed'
	};

	const LOCK_TONES: Record<LockState, StatusTone> = {
		free: 'neutral',
		held: 'warning',
		stale: 'failed'
	};

	const ALERT_TONES: Record<AlertKind, StatusTone> = {
		rpo_violated: 'failed',
		job_failed: 'failed',
		job_dead_letter: 'failed',
		job_warnings: 'warning',
		agent_offline: 'warning',
		stale_lock: 'warning'
	};

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	async function load(): Promise<void> {
		if (!hasTenant) return;
		loading = true;
		error = null;
		try {
			dashboard = await service.getDashboard();
		} catch (problem) {
			error = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	onMount(load);

	function hours(seconds: number): string {
		const minutes = Math.round(seconds / 60);
		return minutes >= 120
			? t('dashboard.hours', { n: Math.round(minutes / 60) })
			: t('dashboard.minutes', { n: minutes });
	}
</script>

<svelte:head>
	<title>{t('overview.title')} · {t('app.name')}</title>
</svelte:head>

<PageHeader title={t('overview.title')} description={t('overview.description')}>
	{#if hasTenant}
		<Button icon="refresh" onclick={load}>{t('common.refresh')}</Button>
	{/if}
</PageHeader>

{#if hasTenant}
	{#if loading && !dashboard}
		<div class="overview" aria-busy="true">
			<Skeleton variant="row" />
			<Skeleton variant="row" />
			<Skeleton variant="row" />
		</div>
	{:else if error && !dashboard}
		<Alert tone="danger" title={error}>
			<Button size="sm" onclick={load}>{t('common.retry')}</Button>
		</Alert>
	{:else if dashboard && firstRun}
		<div class="overview">
			<Alert tone="info" title={t('overview.welcomeTitle')}>{t('overview.welcomeBody')}</Alert>

			<Panel title={t('overview.getStarted')} flush>
				{#snippet actions()}
					<span>{t('overview.progress', { done, total: steps.length })}</span>
				{/snippet}
				<ol class="checklist">
					{#each steps as step, index (step.title)}
						<li class="checklist__item" class:checklist__item--done={step.done}>
							<span class="checklist__mark">
								{#if step.done}
									<Icon name="check" size="xs" />
								{:else}
									{index + 1}
								{/if}
							</span>
							<div class="checklist__main">
								<p class="checklist__title">{step.title}</p>
								<p class="checklist__body">{step.body}</p>
							</div>
							{#if step.action}
								<Button disabled={!step.onAction} onclick={step.onAction}>{step.action}</Button>
							{/if}
						</li>
					{/each}
				</ol>
			</Panel>

			<div class="empties">
				{#each empties as empty (empty.icon)}
					<EmptyState
						icon={empty.icon}
						title={t(empty.title)}
						description={t(empty.body)}
						bordered
					/>
				{/each}
			</div>

			<Panel title={t('overview.how.title')}>
				<ul class="hints">
					{#each hints as hint (hint)}
						<li><Icon name="check" size="xs" />{t(hint)}</li>
					{/each}
				</ul>
			</Panel>
		</div>
	{:else if dashboard}
		<div class="overview">
			{#if error}
				<Alert tone="danger" title={error} />
			{/if}

			<div class="stats">
				<div class="stat">
					<span class="stat__value">{dashboard.hosts.online}</span>
					<span class="stat__label">{t('dashboard.stat.online')}</span>
				</div>
				<div class="stat" class:stat--warning={dashboard.hosts.waiting > 0}>
					<span class="stat__value">{dashboard.hosts.waiting}</span>
					<span class="stat__label">{t('dashboard.stat.waiting')}</span>
				</div>
				<div class="stat" class:stat--warning={dashboard.hosts.offline > 0}>
					<span class="stat__value">{dashboard.hosts.offline}</span>
					<span class="stat__label">{t('dashboard.stat.offline')}</span>
				</div>
				<div class="stat" class:stat--warning={dashboard.plans.at_risk > 0}>
					<span class="stat__value">{dashboard.plans.at_risk}</span>
					<span class="stat__label">{t('dashboard.stat.atRisk')}</span>
				</div>
				<div class="stat" class:stat--danger={dashboard.plans.violated > 0}>
					<span class="stat__value">{dashboard.plans.violated}</span>
					<span class="stat__label">{t('dashboard.stat.violated')}</span>
				</div>
				<div class="stat" class:stat--danger={dashboard.alerts.total > 0}>
					<span class="stat__value">{dashboard.alerts.total}</span>
					<span class="stat__label">{t('dashboard.stat.alerts')}</span>
				</div>
			</div>

			<div class="columns">
				<Panel title={t('dashboard.plans.title')} flush>
					{#snippet actions()}
						<a class="link" href={resolve('/(app)/plans')}>{t('nav.plans')}</a>
					{/snippet}
					{#if dashboard.plans.items.length === 0}
						<p class="empty">{t('dashboard.plans.empty')}</p>
					{:else}
						<table class="list">
							<thead>
								<tr>
									<th>{t('dashboard.plans.plan')}</th>
									<th>{t('dashboard.plans.lastSuccess')}</th>
									<th>{t('dashboard.plans.rpo')}</th>
									<th>{t('dashboard.plans.state')}</th>
								</tr>
							</thead>
							<tbody>
								{#each dashboard.plans.items as plan (plan.id)}
									<tr>
										<td>
											<span class="cell__name">{plan.name}</span>
											<span class="cell__meta">{plan.agent_hostname} · {plan.repository_name}</span>
										</td>
										<td>
											{#if plan.last_success_at}
												<time
													datetime={plan.last_success_at}
													title={formatTime(plan.last_success_at)}
												>
													{relativeTime(plan.last_success_at)}
												</time>
											{:else}
												<span class="cell__meta">{t('dashboard.plans.noSuccess')}</span>
											{/if}
										</td>
										<td>{hours(plan.rpo_seconds)}</td>
										<td>
											{#if plan.enabled}
												<StatusPill tone={RPO_TONES[plan.state]}>
													{t(`rpo.${plan.state}` as MessageKey)}
												</StatusPill>
											{:else}
												<StatusPill tone="neutral">{t('dashboard.plans.paused')}</StatusPill>
											{/if}
										</td>
									</tr>
								{/each}
							</tbody>
						</table>
					{/if}
				</Panel>

				<Panel title={t('dashboard.alerts.title')} flush>
					{#snippet actions()}
						<a class="link" href={resolve('/(app)/alerts')}>{t('dashboard.alerts.viewAll')}</a>
					{/snippet}
					{#if dashboard.alerts.items.length === 0}
						<p class="empty">{t('dashboard.alerts.empty')}</p>
					{:else}
						<ul class="rows">
							{#each dashboard.alerts.items as alert (alert.id)}
								<li class="row">
									<StatusPill tone={ALERT_TONES[alert.kind]}>
										{t(`alerts.kind.${alert.kind}` as MessageKey)}
									</StatusPill>
									<div class="row__main">
										<span class="cell__name">
											{alert.plan_name ?? alert.repository_name ?? alert.agent_hostname}
										</span>
										<span class="cell__meta">{alertCondition(alert)}</span>
									</div>
									<time
										class="cell__meta"
										datetime={alert.opened_at}
										title={formatTime(alert.opened_at)}
									>
										{relativeTime(alert.opened_at)}
									</time>
								</li>
							{/each}
						</ul>
					{/if}
				</Panel>
			</div>

			<div class="columns columns--three">
				<Panel title={t('dashboard.hosts.title')} flush>
					{#snippet actions()}
						<a class="link" href={resolve('/(app)/hosts')}>{t('nav.hosts')}</a>
					{/snippet}
					{#if dashboard.hosts.items.length === 0}
						<p class="empty">{t('dashboard.hosts.empty')}</p>
					{:else}
						<ul class="rows">
							{#each dashboard.hosts.items as host (host.id)}
								<li class="row">
									<StatusPill tone={host.waiting ? 'info' : host.online ? 'success' : 'neutral'}>
										{host.waiting
											? t('dashboard.hosts.waiting')
											: host.online
												? t('dashboard.hosts.online')
												: t('dashboard.hosts.offline')}
									</StatusPill>
									<div class="row__main">
										<span class="cell__name">{host.hostname}</span>
										<span class="cell__meta">
											{host.last_seen_at
												? t('dashboard.hosts.seen', { time: relativeTime(host.last_seen_at) })
												: t('dashboard.hosts.neverSeen')}
										</span>
									</div>
								</li>
							{/each}
						</ul>
					{/if}
				</Panel>

				<Panel title={t('dashboard.repositories.title')} flush>
					{#snippet actions()}
						<a class="link" href={resolve('/(app)/repositories')}>{t('nav.repositories')}</a>
					{/snippet}
					{#if dashboard.repositories.length === 0}
						<p class="empty">{t('dashboard.repositories.empty')}</p>
					{:else}
						<ul class="rows">
							{#each dashboard.repositories as repository (repository.id)}
								<li class="row">
									<StatusPill tone={LOCK_TONES[repository.lock_state]}>
										{t(`lock.${repository.lock_state}` as MessageKey)}
									</StatusPill>
									<div class="row__main">
										<span class="cell__name">{repository.name}</span>
									</div>
								</li>
							{/each}
						</ul>
					{/if}
				</Panel>

				<Panel title={t('dashboard.failedJobs.title')} flush>
					{#snippet actions()}
						<a class="link" href={resolve('/(app)/jobs')}>{t('nav.jobs')}</a>
					{/snippet}
					{#if dashboard.failed_jobs.length === 0}
						<p class="empty">{t('dashboard.failedJobs.empty')}</p>
					{:else}
						<ul class="rows">
							{#each dashboard.failed_jobs as job (job.id)}
								<li class="row">
									<StatusPill tone="failed">
										{t(`jobs.status.${job.status}` as MessageKey)}
									</StatusPill>
									<div class="row__main">
										<a class="cell__name" href={resolve('/(app)/jobs/[id]', { id: job.id })}>
											{job.plan_name ?? job.repository_name ?? job.agent_hostname}
										</a>
										<span class="cell__meta">{job.agent_hostname}</span>
									</div>
									<time
										class="cell__meta"
										datetime={job.finished_at}
										title={formatTime(job.finished_at)}
									>
										{relativeTime(job.finished_at)}
									</time>
								</li>
							{/each}
						</ul>
					{/if}
				</Panel>
			</div>
		</div>
	{/if}
{:else}
	<Card>
		<EmptyState title={t('overview.title')} description={t('overview.noOrganizations')} bordered />
	</Card>
{/if}

<style>
	.overview {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-6);
	}

	.checklist {
		margin: 0;
		padding: 0;
		list-style: none;
	}

	.checklist__item {
		display: flex;
		align-items: flex-start;
		gap: var(--rh-space-3);
		padding: var(--rh-space-4);
		border-top: var(--rh-border-width) solid var(--rh-color-border-muted);
	}

	.checklist__item:first-child {
		border-top: 0;
	}

	.checklist__item--done {
		color: var(--rh-color-text-muted);
	}

	.checklist__mark {
		display: inline-flex;
		flex: none;
		align-items: center;
		justify-content: center;
		width: 22px;
		height: 22px;
		margin-top: 1px;
		border: var(--rh-border-width) solid var(--rh-color-border-strong);
		border-radius: var(--rh-radius-pill);
		color: var(--rh-color-text-subtle);
		font-size: var(--rh-font-size-sm);
	}

	.checklist__item--done .checklist__mark {
		border-color: var(--rh-color-success-subtle-border);
		background-color: var(--rh-color-success-subtle);
		color: var(--rh-color-success-text);
	}

	.checklist__main {
		flex: 1 1 auto;
		min-width: 0;
	}

	.checklist__title {
		margin: 0;
		font-weight: var(--rh-font-weight-semibold);
	}

	.checklist__body {
		margin: 2px 0 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.empties {
		display: grid;
		grid-template-columns: repeat(3, minmax(0, 1fr));
		gap: var(--rh-space-4);
	}

	@media (max-width: 900px) {
		.empties {
			grid-template-columns: minmax(0, 1fr);
		}
	}

	.hints {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-2);
		margin: 0;
		padding: 0;
		list-style: none;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.hints li {
		display: flex;
		align-items: flex-start;
		gap: var(--rh-space-2);
	}

	.hints :global(.icon) {
		margin-top: 2px;
		color: var(--rh-color-text-subtle);
	}

	.stats {
		display: grid;
		grid-template-columns: repeat(6, minmax(0, 1fr));
		gap: var(--rh-space-4);
	}

	.stat {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-1);
		padding: var(--rh-space-4);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-surface);
	}

	.stat__value {
		font-size: var(--rh-font-size-2xl);
		font-weight: var(--rh-font-weight-semibold);
		line-height: 1.1;
	}

	.stat__label {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.stat--warning .stat__value {
		color: var(--rh-color-warning-text);
	}

	.stat--danger .stat__value {
		color: var(--rh-color-danger-text);
	}

	.columns {
		display: grid;
		grid-template-columns: minmax(0, 3fr) minmax(0, 2fr);
		gap: var(--rh-space-4);
		align-items: start;
	}

	.columns--three {
		grid-template-columns: repeat(3, minmax(0, 1fr));
	}

	@media (max-width: 1100px) {
		.stats {
			grid-template-columns: repeat(2, minmax(0, 1fr));
		}

		.columns,
		.columns--three {
			grid-template-columns: minmax(0, 1fr);
		}
	}

	.link {
		color: var(--rh-color-link);
		font-size: var(--rh-font-size-sm);
		text-decoration: none;
	}

	.link:hover {
		color: var(--rh-color-link-hover);
		text-decoration: underline;
	}

	.empty {
		margin: 0;
		padding: var(--rh-space-4);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.list {
		width: 100%;
		border-collapse: collapse;
		font-size: var(--rh-font-size-sm);
	}

	.list th {
		padding: var(--rh-space-2) var(--rh-space-4);
		color: var(--rh-color-text-subtle);
		font-weight: var(--rh-font-weight-semibold);
		text-align: left;
	}

	.list td {
		padding: var(--rh-space-3) var(--rh-space-4);
		border-top: var(--rh-border-width) solid var(--rh-color-border-muted);
		vertical-align: top;
	}

	.cell__name {
		display: block;
		color: var(--rh-color-text);
		font-weight: var(--rh-font-weight-semibold);
		text-decoration: none;
	}

	a.cell__name:hover {
		color: var(--rh-color-link-hover);
		text-decoration: underline;
	}

	.cell__meta {
		display: block;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.rows {
		margin: 0;
		padding: 0;
		list-style: none;
	}

	.row {
		display: flex;
		align-items: flex-start;
		gap: var(--rh-space-3);
		padding: var(--rh-space-3) var(--rh-space-4);
		border-top: var(--rh-border-width) solid var(--rh-color-border-muted);
	}

	.row:first-child {
		border-top: 0;
	}

	.row__main {
		flex: 1 1 auto;
		min-width: 0;
	}
</style>
