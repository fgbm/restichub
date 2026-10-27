<script lang="ts">
	import { onMount } from 'svelte';
	import { coalesce } from '#lib/events.ts';
	import { isStreamConnected, onEvent } from '#lib/stores/events.svelte.ts';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { Dashboard, JobResultCode } from '#lib/api/index.ts';
	import { Page } from '#lib/components/layout/index.ts';
	import {
		Alert,
		Button,
		Card,
		EmptyState,
		Icon,
		ListView,
		Panel,
		Skeleton
	} from '#lib/components/ui/index.ts';
	import type { IconName, ListMeta } from '#lib/components/ui/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { ago, dayTime } from '#lib/list-format.ts';
	import {
		alertPill,
		alertState,
		iconOf,
		jobState,
		lockState,
		overviewHostState,
		pillOf,
		rpoPlanState
	} from '#lib/vocabulary.ts';
	import { isSingleTenant } from '#lib/stores/instance.svelte.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { showSkeleton } from '#lib/skeleton.svelte.ts';

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
	const skeleton = showSkeleton(() => loading && !dashboard);

	// An organization with no host, no plan and no repository still shows the first steps.
	const firstRun = $derived(
		dashboard !== null &&
			dashboard.hosts.total === 0 &&
			dashboard.plans.total === 0 &&
			dashboard.repositories.length === 0
	);

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	async function load(silent = false): Promise<void> {
		if (!hasTenant) return;
		if (!silent) loading = true;
		error = null;
		try {
			dashboard = await service.getDashboard();
		} catch (problem) {
			error = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	onMount(() => void load());

	// The server caches the facts for a short time, so the page reads them at most once a second.
	const reloadSoon = coalesce(() => load(true), 1000);
	onMount(() =>
		onEvent((event) => {
			if (event.type !== 'job.progress' && event.type !== 'job.log') reloadSoon();
		})
	);

	$effect(() => {
		const timer = setInterval(
			() => void (!document.hidden && load(true)),
			isStreamConnected() ? 120_000 : 30_000
		);
		return () => clearInterval(timer);
	});

	type DashboardPlan = Dashboard['plans']['items'][number];
	type DashboardAlert = Dashboard['alerts']['items'][number];
	type DashboardHost = Dashboard['hosts']['items'][number];
	type DashboardRepository = Dashboard['repositories'][number];
	type FailedJob = Dashboard['failed_jobs'][number];

	function planMeta(plan: DashboardPlan): ListMeta[] {
		return [
			plan.agent_hostname,
			plan.repository_name,
			plan.last_success_at
				? {
						text: t('plans.meta.lastBackup', { time: ago(plan.last_success_at) }),
						time: plan.last_success_at
					}
				: t('plans.meta.noBackup')
		];
	}

	function alertMeta(alert: DashboardAlert): ListMeta[] {
		return [
			{ text: t('alerts.meta.opened', { time: ago(alert.opened_at) }), time: alert.opened_at },
			alert.agent_hostname ?? null
		];
	}

	function hostMeta(host: DashboardHost): ListMeta[] {
		return [
			host.last_seen_at
				? {
						text: t('hosts.meta.contact', { time: ago(host.last_seen_at) }),
						time: host.last_seen_at
					}
				: t('hosts.meta.neverSeen')
		];
	}

	function failedState(job: FailedJob) {
		return jobState({ status: job.status, result_code: job.result_code as JobResultCode | null });
	}

	function jobMeta(job: FailedJob): ListMeta[] {
		return [
			job.agent_hostname,
			{ text: dayTime(job.finished_at), time: job.finished_at },
			job.plan_name ? t(`jobs.kind.${job.kind}` as MessageKey) : null,
			job.repository_name ?? null
		];
	}

	function repositoryHref(repository: DashboardRepository): string {
		return resolve('/(app)/repositories/[id]', { id: repository.id });
	}
</script>

<Page title={t('overview.title')} description={t('overview.description')}>
	{#snippet actions()}
		{#if hasTenant}
			<Button icon="refresh" onclick={() => load()}>{t('common.refresh')}</Button>
		{/if}
	{/snippet}

	{#if hasTenant}
		{#if loading && !dashboard}
			{#if skeleton()}
				<div class="overview" aria-busy="true">
					<Skeleton variant="row" />
					<Skeleton variant="row" />
					<Skeleton variant="row" />
				</div>
			{/if}
		{:else if error && !dashboard}
			<Alert tone="danger" title={error}>
				<Button size="sm" onclick={() => load()}>{t('common.retry')}</Button>
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
							<ListView
								flush
								paginated={false}
								label={t('dashboard.plans.title')}
								items={dashboard.plans.items}
								itemKey={(plan) => plan.id}
								icon={(plan) => iconOf(rpoPlanState(plan))}
								pill={(plan) => pillOf(rpoPlanState(plan))}
								title={(plan) => plan.name}
								href={(plan) => resolve('/(app)/plans/[id]', { id: plan.id })}
								meta={planMeta}
							/>
						{/if}
					</Panel>

					<Panel title={t('dashboard.alerts.title')} flush>
						{#snippet actions()}
							<a class="link" href={resolve('/(app)/alerts')}>{t('dashboard.alerts.viewAll')}</a>
						{/snippet}
						{#if dashboard.alerts.items.length === 0}
							<p class="empty">{t('dashboard.alerts.empty')}</p>
						{:else}
							<ListView
								flush
								paginated={false}
								label={t('dashboard.alerts.title')}
								items={dashboard.alerts.items}
								itemKey={(alert) => alert.id}
								icon={(alert) => iconOf(alertState(alert))}
								pill={alertPill}
								title={(alert) =>
									t('alerts.title.item', {
										kind: t(`alerts.kind.${alert.kind}` as MessageKey),
										subject: alert.plan_name ?? alert.repository_name ?? alert.agent_hostname ?? ''
									})}
								href={(alert) =>
									alert.job_id ? resolve('/(app)/jobs/[id]', { id: alert.job_id }) : undefined}
								meta={alertMeta}
							/>
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
							<ListView
								flush
								paginated={false}
								label={t('dashboard.hosts.title')}
								items={dashboard.hosts.items}
								itemKey={(host) => host.id}
								icon={(host) => iconOf(overviewHostState(host))}
								pill={(host) => pillOf(overviewHostState(host))}
								title={(host) => host.hostname}
								href={(host) => resolve('/(app)/hosts/[id]', { id: host.id })}
								meta={hostMeta}
							/>
						{/if}
					</Panel>

					<Panel title={t('dashboard.repositories.title')} flush>
						{#snippet actions()}
							<a class="link" href={resolve('/(app)/repositories')}>{t('nav.repositories')}</a>
						{/snippet}
						{#if dashboard.repositories.length === 0}
							<p class="empty">{t('dashboard.repositories.empty')}</p>
						{:else}
							<ListView
								flush
								paginated={false}
								label={t('dashboard.repositories.title')}
								items={dashboard.repositories}
								itemKey={(repository) => repository.id}
								icon={(repository) => iconOf(lockState(repository.lock_state))}
								pill={(repository) => pillOf(lockState(repository.lock_state))}
								title={(repository) => repository.name}
								href={repositoryHref}
							/>
						{/if}
					</Panel>

					<Panel title={t('dashboard.failedJobs.title')} flush>
						{#snippet actions()}
							<a class="link" href={resolve('/(app)/jobs')}>{t('nav.jobs')}</a>
						{/snippet}
						{#if dashboard.failed_jobs.length === 0}
							<p class="empty">{t('dashboard.failedJobs.empty')}</p>
						{:else}
							<ListView
								flush
								paginated={false}
								label={t('dashboard.failedJobs.title')}
								items={dashboard.failed_jobs}
								itemKey={(job) => job.id}
								icon={(job) => iconOf(failedState(job))}
								pill={(job) => pillOf(failedState(job))}
								title={(job) => job.plan_name ?? t(`jobs.kind.${job.kind}` as MessageKey)}
								href={(job) => resolve('/(app)/jobs/[id]', { id: job.id })}
								meta={jobMeta}
							/>
						{/if}
					</Panel>
				</div>
			</div>
		{/if}
	{:else}
		<Card>
			<EmptyState
				title={t('overview.title')}
				description={t('overview.noOrganizations')}
				bordered
			/>
		</Card>
	{/if}
</Page>

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
</style>
