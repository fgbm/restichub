<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { JobDetail, JobLogLine, JobStep } from '#lib/api/index.ts';
	import LogViewer from '#lib/components/LogViewer.svelte';
	import {
		Alert,
		Button,
		ConfirmDialog,
		EmptyState,
		Icon,
		Panel,
		Skeleton,
		StatusPill
	} from '#lib/components/ui/index.ts';
	import type { StatusTone } from '#lib/components/ui/index.ts';
	import {
		formatBytes,
		formatCount,
		formatDuration,
		formatLocalTime,
		formatSeconds
	} from '#lib/format.ts';
	import { isLive, limitText, markerOf, statsOf, stepProgress, stepRows } from '#lib/job-detail.ts';
	import type { StepRow } from '#lib/job-detail.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	const POLL_MS = 2000;
	const LOG_PAGE = 1000;
	// The viewer holds this many lines at most. A bigger log shows its first lines.
	const MAX_LINES = 10_000;

	const MARKER_ICONS = {
		success: 'check',
		warning: 'alert',
		failed: 'x',
		idle: 'pause',
		running: 'dot'
	} as const;

	const id = $derived(page.params.id ?? '');

	let job = $state<JobDetail | null>(null);
	let loading = $state(true);
	let notFound = $state(false);
	let error = $state<string | null>(null);

	let lines = $state<JobLogLine[]>([]);
	let logTruncated = $state(false);
	let logPurged = $state(false);
	let logError = $state(false);
	let following = $state(true);

	let cancelOpen = $state(false);
	let cancelling = $state(false);
	let rerunning = $state(false);
	let busy = false;

	const canWrite = $derived(['admin', 'operator'].includes(getSession()?.role ?? ''));
	const live = $derived(job ? isLive(job.status) : false);
	const cancelled = $derived(job?.status === 'failed' && job.result_code === 'cancelled');
	const warnings = $derived(
		job?.status === 'succeeded' && job.result_code === 'succeeded_with_warnings'
	);
	const stats = $derived(statsOf(job?.summary));
	const capped = $derived(lines.length >= MAX_LINES);
	const rows = $derived(job ? stepRows(job) : []);
	const progress = $derived(job ? stepProgress(job, rows) : null);
	const runningProgress = $derived(
		job?.progress &&
			rows.some((row) => row.state === 'running' && row.id === job?.progress?.step_id)
			? job.progress
			: null
	);

	const statusLabel = $derived.by(() => {
		if (!job) return '';
		if (warnings) return t('jobs.status.succeeded_with_warnings');
		return t(`jobs.status.${cancelled ? 'cancelled' : job.status}` as MessageKey);
	});

	const tone = $derived.by((): StatusTone => {
		if (!job) return 'neutral';
		if (cancelled) return 'cancelled';
		if (warnings) return 'warning';
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
	});

	const title = $derived(
		job ? (job.plan_name ?? t(`jobs.kind.${job.kind}` as MessageKey)) : t('jobs.title')
	);

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	function reasonOf(current: JobDetail): string | null {
		if (cancelled) return null;
		const code =
			current.result_message ??
			(current.result_code === 'repository_locked' ? current.result_code : null);
		if (!code) return null;
		const key = `jobs.reason.${code}` as MessageKey;
		const text = t(key);
		return text === key ? code : text;
	}

	function stepTitle(stepId: string): string {
		const key = `jobs.kind.${stepId.replaceAll('-', '_')}` as MessageKey;
		const text = t(key);
		return text === key ? stepId : text;
	}

	function stepError(step: JobStep): string | null {
		if (!step.error_code) return null;
		const key = `jobDetail.stepError.${step.error_code}` as MessageKey;
		const text = t(key);
		return text === key ? step.error_code : text;
	}

	// The duration is the own time of the step: its start and end come from the host.
	function stepMeta(step: JobStep): string {
		const parts = [t(`jobDetail.stepStatus.${step.status}` as MessageKey)];
		if (step.started_at && step.finished_at && step.status !== 'skipped') {
			parts.push(formatDuration(step.started_at, step.finished_at));
		}
		if (step.exit_code >= 0) parts.push(t('jobDetail.exitCode', { code: String(step.exit_code) }));
		return parts.join(' · ');
	}

	function rowMeta(row: StepRow): string {
		if (row.step) return stepMeta(row.step);
		if (row.state === 'running') {
			const elapsed = runningProgress ? ` · ${formatSeconds(runningProgress.seconds_elapsed)}` : '';
			return `${t('jobDetail.stepRunning')}${elapsed}`;
		}
		return t(live ? 'jobDetail.stepQueued' : 'jobDetail.stepNotRun');
	}

	function statValue(stat: ReturnType<typeof statsOf>[number]): string {
		return stat.params
			? t('jobDetail.stat.processedValue', { ...stat.params, size: stat.value })
			: stat.value;
	}

	async function loadJob(): Promise<void> {
		try {
			job = await service.getJob(id);
			error = null;
			notFound = false;
		} catch (problem) {
			if (problem instanceof ApiError && problem.status === 404) notFound = true;
			else error = messageOf(problem);
		}
	}

	// Reads the lines after the last held line until the log ends or the viewer is full.
	async function loadLines(): Promise<void> {
		try {
			// The viewer is full before the next request, so a full log asks for nothing.
			while (lines.length < MAX_LINES) {
				const after = lines.length > 0 ? lines[lines.length - 1].seq : 0;
				const limit = Math.min(LOG_PAGE, MAX_LINES - lines.length);
				const result = await service.getJobLogs(id, { after_seq: after, limit });
				logTruncated = result.truncated;
				logPurged = result.purged;
				logError = false;
				// A line is held once, whatever the server returns.
				const fresh = result.items.filter((item) => item.seq > after);
				if (fresh.length > 0) lines = [...lines, ...fresh];
				if (!result.has_more || fresh.length === 0 || lines.length >= MAX_LINES) break;
			}
		} catch {
			logError = true;
		}
	}

	async function refresh(): Promise<void> {
		if (busy) return;
		busy = true;
		try {
			await loadJob();
			if (job && !notFound) await loadLines();
		} finally {
			busy = false;
			loading = false;
		}
	}

	onMount(() => {
		void refresh();
	});

	$effect(() => {
		if (!live) return;
		const timer = setInterval(() => void refresh(), POLL_MS);
		return () => clearInterval(timer);
	});

	async function copyLog(): Promise<void> {
		const text = lines
			.map((line) => `${line.ts} ${line.level.toUpperCase()} ${line.text}`)
			.join('\n');
		try {
			await navigator.clipboard.writeText(text);
			pushToast('success', t('jobDetail.logCopied'));
		} catch {
			pushToast('danger', t('jobDetail.logCopyFailed'));
		}
	}

	async function runAgain(): Promise<void> {
		if (!job?.plan_id || rerunning) return;
		rerunning = true;
		try {
			const started = await service.runPlan(job.plan_id);
			pushToast('success', t('plans.runQueued'));
			await goto(resolve('/(app)/jobs/[id]', { id: started.id }));
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			rerunning = false;
		}
	}

	async function confirmCancel(): Promise<void> {
		cancelling = true;
		try {
			const result = await service.cancelJob(id);
			const done =
				result.status === 'queued' ||
				(result.status === 'failed' && result.result_code === 'cancelled');
			pushToast('success', t(done ? 'jobs.cancelled' : 'jobs.cancelRequested'));
			cancelOpen = false;
			await refresh();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			cancelling = false;
		}
	}
</script>

<svelte:head>
	<title>{title} · {t('jobs.title')} · {t('app.name')}</title>
</svelte:head>

{#if loading}
	<Skeleton variant="row" />
	<div class="gap"></div>
	<Skeleton variant="row" />
{:else if notFound}
	<EmptyState icon="job" title={t('jobDetail.notFound')} bordered />
{:else if error && !job}
	<Alert tone="danger" title={error}></Alert>
{:else if job}
	<header class="head">
		<div class="head__text">
			<div class="head__title-row">
				<h1 class="head__title mono">{job.id}</h1>
				<StatusPill {tone}>{statusLabel}</StatusPill>
				{#if live}<span class="head__live">{t('jobDetail.live')}</span>{/if}
			</div>
			<p class="head__subtitle">
				{#if job.plan_name}
					{t('jobs.plan')}
					<a href={resolve('/(app)/plans')}>{job.plan_name}</a>
					·
				{:else}
					{t(`jobs.kind.${job.kind}` as MessageKey)}
					·
				{/if}
				{t('jobs.host').toLowerCase()}
				<a href={resolve('/(app)/hosts')}>{job.agent_hostname ?? t('common.none')}</a>
				· {t(`jobDetail.triggeredBy.${job.trigger}` as MessageKey)}
				{#if progress && progress.total > 0 && live}
					· {t('jobDetail.stepOf', { current: progress.position, total: progress.total })}
				{/if}
			</p>
			{#if reasonOf(job)}
				<p class="head__reason">{reasonOf(job)}</p>
			{/if}
		</div>
		<div class="head__actions">
			{#if canWrite && job.plan_id}
				<Button disabled={live} loading={rerunning} onclick={runAgain}>
					{t('jobDetail.runAgain')}
				</Button>
			{/if}
			{#if canWrite && live}
				<Button variant="danger" icon="stop" onclick={() => (cancelOpen = true)}>
					{t('jobs.cancel')}
				</Button>
			{/if}
		</div>
	</header>

	<Panel title={t('jobDetail.summary')} subtitle={t('jobDetail.summaryHint')}>
		{#if stats.length === 0}
			<p class="muted">{t('jobDetail.summaryEmpty')}</p>
		{:else}
			<div class="stats">
				{#each stats as stat (stat.label)}
					<div class="stat">
						<span class="stat__value" class:stat__value--mono={stat.mono}>{statValue(stat)}</span>
						<span class="stat__label">{t(stat.label)}</span>
					</div>
				{/each}
			</div>
		{/if}
	</Panel>

	<div class="layout">
		<div class="layout__main">
			<Panel
				title={t('jobDetail.steps')}
				subtitle={progress && progress.total > 0
					? t('jobDetail.stepsComplete', { done: progress.complete, total: progress.total })
					: undefined}
			>
				{#if rows.length === 0}
					<p class="muted">{t('jobDetail.stepsEmpty')}</p>
				{:else}
					{#if progress}
						<progress
							class="progress progress--steps"
							aria-label={t('jobDetail.steps')}
							max="1"
							value={progress.fraction}
						></progress>
					{/if}
					<ol class="timeline">
						{#each rows as row (row.id)}
							<li class="timeline__item">
								{#if row.state === 'running'}
									<span class="timeline__marker timeline__marker--running"></span>
								{:else}
									{@const marker = row.step ? markerOf(row.step.status) : 'idle'}
									<span class="timeline__marker timeline__marker--{marker}">
										<Icon name={MARKER_ICONS[marker]} size="xs" />
									</span>
								{/if}
								<div class="timeline__head">
									<span class="timeline__title">{stepTitle(row.id)}</span>
									<span class="timeline__meta">{rowMeta(row)}</span>
								</div>
								{#if row.step}
									{#if stepError(row.step)}
										<p class="timeline__body">{stepError(row.step)}</p>
									{/if}
									{#if row.step.possible_stale_lock}
										<p class="timeline__body">{t('jobDetail.staleLock')}</p>
									{/if}
									{#if statsOf(row.step.output).length > 0}
										<p class="timeline__body">
											{#each statsOf(row.step.output).slice(0, 3) as stat, index (stat.label)}
												{index > 0 ? ' · ' : ''}{t(stat.label)}: {statValue(stat)}
											{/each}
										</p>
									{/if}
								{:else if row.state === 'running' && runningProgress}
									<progress
										class="progress"
										aria-label={t('jobDetail.progress')}
										max="1"
										value={runningProgress.percent_done}
									></progress>
									<p class="timeline__body">
										{#if runningProgress.total_files > 0}
											{t('jobDetail.progressFiles', {
												done: formatCount(runningProgress.files_done),
												total: formatCount(runningProgress.total_files)
											})}
											·
										{/if}
										{t('jobDetail.progressBytes', {
											done: formatBytes(runningProgress.bytes_done),
											total: formatBytes(runningProgress.total_bytes)
										})}
										{#if runningProgress.seconds_remaining > 0}
											· {t('jobDetail.progressRemaining', {
												time: formatSeconds(runningProgress.seconds_remaining)
											})}
										{/if}
									</p>
									{#if runningProgress.current_files.length > 0}
										<p class="timeline__body timeline__body--mono">
											{t('jobDetail.progressCurrent', { file: runningProgress.current_files[0] })}
										</p>
									{/if}
								{/if}
							</li>
						{/each}
					</ol>
				{/if}
			</Panel>

			<div class="gap"></div>
			{#if logPurged}
				<Alert tone="info" title={t('jobDetail.logPurged')}></Alert>
				<div class="gap"></div>
			{/if}
			<LogViewer
				title={live ? t('jobDetail.logLive') : t('jobDetail.log')}
				{lines}
				{live}
				bind:following
				emptyText={t('jobDetail.logEmpty')}
				onCopy={copyLog}
			/>
			{#if logTruncated}
				<p class="muted note">{t('jobDetail.logTruncated')}</p>
			{/if}
			{#if capped}
				<p class="muted note">{t('jobDetail.logCapped', { count: formatCount(MAX_LINES) })}</p>
			{/if}
			{#if logError}
				<p class="note note--error">{t('jobDetail.logError')}</p>
			{/if}
		</div>

		<aside class="layout__side">
			<Panel title={t('jobDetail.about')}>
				<dl class="about">
					<dt>{t('jobDetail.id')}</dt>
					<dd class="mono">{job.id}</dd>
					<dt>{t('jobDetail.kind')}</dt>
					<dd>{t(`jobs.kind.${job.kind}` as MessageKey)}</dd>
					{#if job.plan_name}
						<dt>{t('jobs.plan')}</dt>
						<dd><a href={resolve('/(app)/plans')}>{job.plan_name}</a></dd>
					{/if}
					<dt>{t('jobs.host')}</dt>
					<dd>
						<a href={resolve('/(app)/hosts')}>{job.agent_hostname ?? t('common.none')}</a>
					</dd>
					{#if job.repository_name}
						<dt>{t('jobs.repository')}</dt>
						<dd><a href={resolve('/(app)/repositories')}>{job.repository_name}</a></dd>
					{/if}
					<dt>{t('jobDetail.trigger')}</dt>
					<dd>{t(`jobDetail.trigger.${job.trigger}` as MessageKey)}</dd>
					<dt>{t('jobDetail.queuedAt')}</dt>
					<dd>{formatLocalTime(job.created_at)}</dd>
					{#if job.started_at}
						<dt>{t('jobDetail.startedAt')}</dt>
						<dd>{formatLocalTime(job.started_at)}</dd>
					{/if}
					{#if job.finished_at}
						<dt>{t('jobDetail.finishedAt')}</dt>
						<dd>{formatLocalTime(job.finished_at)}</dd>
					{/if}
					{#if job.expires_at && (job.status === 'queued' || job.status === 'sent')}
						<dt>{t('jobDetail.acceptUntil')}</dt>
						<dd>{formatLocalTime(job.expires_at)}</dd>
					{/if}
					{#if limitText(job)}
						<dt>{t('jobDetail.limit')}</dt>
						<dd>{limitText(job)}</dd>
					{/if}
					{#if job.attempt > 0}
						<dt>{t('jobDetail.attempt')}</dt>
						<dd>{job.attempt + 1}</dd>
					{/if}
				</dl>
			</Panel>
		</aside>
	</div>
{/if}

<ConfirmDialog
	bind:open={cancelOpen}
	title={t('jobs.cancelTitle')}
	message={t('jobs.cancelBody', { name: title })}
	confirmLabel={t('jobs.cancel')}
	cancelLabel={t('jobs.cancelKeep')}
	tone="danger"
	loading={cancelling}
	onConfirm={confirmCancel}
/>

<style>
	.gap {
		height: var(--rh-space-4);
	}

	.mono {
		font-family: var(--rh-font-mono);
	}

	.muted {
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.head {
		display: flex;
		align-items: flex-start;
		gap: var(--rh-space-4);
		padding-bottom: var(--rh-space-6);
	}

	.head__text {
		min-width: 0;
		flex: 1;
	}

	.head__title-row {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--rh-space-3);
	}

	.head__title {
		margin: 0;
		font-size: var(--rh-font-size-xl);
		font-weight: var(--rh-font-weight-semibold);
		line-height: var(--rh-line-height-tight);
	}

	.head__live {
		padding: 0 var(--rh-space-2);
		border: var(--rh-border-width) solid var(--rh-color-running-subtle-border);
		border-radius: var(--rh-radius-pill);
		background-color: var(--rh-color-running-subtle);
		color: var(--rh-color-running-subtle-text);
		font-size: var(--rh-font-size-xs);
	}

	.head__subtitle {
		margin: var(--rh-space-2) 0 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
		overflow-wrap: anywhere;
	}

	.head__reason {
		margin: var(--rh-space-2) 0 0;
		color: var(--rh-color-danger-text);
		font-size: var(--rh-font-size-sm);
	}

	.head__actions {
		display: flex;
		flex: none;
		gap: var(--rh-space-2);
	}

	.stats {
		display: grid;
		grid-template-columns: repeat(4, minmax(0, 1fr));
		gap: var(--rh-space-5) var(--rh-space-4);
	}

	.stat {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-1);
		min-width: 0;
	}

	.stat__value {
		font-size: var(--rh-font-size-xl);
		font-weight: var(--rh-font-weight-semibold);
		font-variant-numeric: tabular-nums;
		overflow-wrap: anywhere;
	}

	.stat__value--mono {
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-md);
	}

	.stat__label {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.layout {
		display: grid;
		grid-template-columns: minmax(0, 1fr) 300px;
		gap: var(--rh-space-8);
		align-items: start;
		margin-top: var(--rh-space-6);
	}

	@media (max-width: 1000px) {
		.layout {
			grid-template-columns: minmax(0, 1fr);
		}

		.stats {
			grid-template-columns: repeat(2, minmax(0, 1fr));
		}
	}

	.timeline {
		position: relative;
		margin: 0;
		padding: 0 0 0 var(--rh-space-6);
		list-style: none;
	}

	.timeline__item {
		position: relative;
		padding-bottom: var(--rh-space-5);
	}

	.timeline__item:last-child {
		padding-bottom: 0;
	}

	.timeline__item::before {
		content: '';
		position: absolute;
		inset-inline-start: -14px;
		top: 18px;
		bottom: -6px;
		width: 2px;
		background-color: var(--rh-color-border);
	}

	.timeline__item:last-child::before {
		display: none;
	}

	.timeline__marker {
		position: absolute;
		inset-inline-start: -22px;
		top: 1px;
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 18px;
		height: 18px;
		box-sizing: border-box;
		border: 2px solid var(--rh-color-border-strong);
		border-radius: 50%;
		background-color: var(--rh-color-canvas);
		color: var(--rh-color-text-muted);
	}

	.timeline__marker--success {
		border-color: var(--rh-color-success);
		color: var(--rh-color-success);
	}

	.timeline__marker--warning {
		border-color: var(--rh-color-warning);
		color: var(--rh-color-warning);
	}

	.timeline__marker--failed {
		border-color: var(--rh-color-danger);
		color: var(--rh-color-danger);
	}

	.timeline__marker--running {
		border-color: var(--rh-color-running);
	}

	.timeline__head {
		display: flex;
		align-items: baseline;
		gap: var(--rh-space-3);
	}

	.timeline__title {
		font-weight: var(--rh-font-weight-semibold);
	}

	.timeline__meta {
		margin-inline-start: auto;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
		text-align: end;
	}

	.timeline__body {
		margin: 2px 0 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
		overflow-wrap: anywhere;
	}

	.timeline__body--mono {
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-xs);
	}

	.progress {
		display: block;
		width: 100%;
		height: 6px;
		margin: var(--rh-space-2) 0;
		border: 0;
		border-radius: var(--rh-radius-pill);
		background-color: var(--rh-color-canvas-inset);
		overflow: hidden;
		appearance: none;
	}

	.progress--steps {
		margin: 0 0 var(--rh-space-5);
	}

	.progress::-webkit-progress-bar {
		background-color: var(--rh-color-canvas-inset);
	}

	.progress::-webkit-progress-value {
		border-radius: var(--rh-radius-pill);
		background-color: var(--rh-color-accent);
	}

	.progress::-moz-progress-bar {
		border-radius: var(--rh-radius-pill);
		background-color: var(--rh-color-accent);
	}

	.note {
		margin: var(--rh-space-2) 0 0;
		font-size: var(--rh-font-size-sm);
	}

	.note--error {
		color: var(--rh-color-danger-text);
	}

	.about {
		display: grid;
		grid-template-columns: minmax(80px, auto) minmax(0, 1fr);
		margin: 0;
	}

	.about dt,
	.about dd {
		margin: 0;
		padding: var(--rh-space-2) 0;
		border-top: var(--rh-border-width) solid var(--rh-color-border-muted);
		font-size: var(--rh-font-size-base);
		overflow-wrap: anywhere;
	}

	.about dt {
		padding-inline-end: var(--rh-space-4);
		color: var(--rh-color-text-muted);
	}

	.about dt:first-of-type,
	.about dd:first-of-type {
		border-top: 0;
	}
</style>
