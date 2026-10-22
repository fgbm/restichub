<script lang="ts">
	import RestoreParams from '#lib/components/RestoreParams.svelte';
	import { onMount } from 'svelte';
	import { coalesce } from '#lib/events.ts';
	import { isStreamConnected, onEvent } from '#lib/stores/events.svelte.ts';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { JobDetail, JobLogLine } from '#lib/api/index.ts';
	import JobSteps from '#lib/components/JobSteps.svelte';
	import StepLog from '#lib/components/StepLog.svelte';
	import { RecordPage } from '#lib/components/layout/index.ts';
	import { Alert, Button, ConfirmDialog, Panel, StatusPill } from '#lib/components/ui/index.ts';
	import type { StatusTone } from '#lib/components/ui/index.ts';
	import { formatCount, formatLocalTime } from '#lib/format.ts';
	import {
		isLive,
		limitLines,
		shortKeyId,
		statText,
		statsOf,
		stepProgress,
		stepRows
	} from '#lib/job-detail.ts';
	import { groupLines, logText } from '#lib/job-log.ts';
	import { jobDetailReason } from '#lib/job-reason.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';
	import { sameData } from '#lib/unchanged.ts';

	const POLL_MS = 2000;
	// The stream pushes the changes, so a connected page polls only as a safety net.
	const STREAM_POLL_MS = 30_000;
	const EVENT_WAIT_MS = 150;
	const LOG_PAGE = 1000;
	const DOWNLOAD_PAGE = 2000;
	// The viewer holds this many lines at most. A bigger log shows its first lines.
	const MAX_LINES = 10_000;

	const { jobId }: { jobId: string } = $props();

	let job = $state<JobDetail | null>(null);
	let loading = $state(true);
	let notFound = $state(false);
	let error = $state<string | null>(null);

	let lines = $state<JobLogLine[]>([]);
	let logTruncated = $state(false);
	let logPurged = $state(false);
	let logError = $state(false);
	let downloading = $state(false);

	let cancelOpen = $state(false);
	let cancelling = $state(false);
	let rerunning = $state(false);
	let keyCopied = $state(false);
	let idCopied = $state(false);
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
	const groups = $derived(
		groupLines(
			lines,
			rows.map((row) => row.id)
		)
	);
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

	const subtitle = $derived.by(() => {
		if (!job) return '';
		const parts = [
			job.plan_name
				? `${t('jobs.plan')} ${job.plan_name}`
				: t(`jobs.kind.${job.kind}` as MessageKey),
			`${t('jobs.host').toLowerCase()} ${job.agent_hostname ?? t('common.none')}`,
			t(`jobDetail.triggeredBy.${job.trigger}` as MessageKey)
		];
		if (progress && progress.total > 0 && live) {
			parts.push(t('jobDetail.stepOf', { current: progress.position, total: progress.total }));
		}
		return parts.join(' · ');
	});

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	async function copyKeyId(keyId: string): Promise<void> {
		keyCopied = await copyText(keyId);
		if (keyCopied) setTimeout(() => (keyCopied = false), 1500);
	}

	async function copyId(jobId: string): Promise<void> {
		idCopied = await copyText(jobId);
		if (idCopied) setTimeout(() => (idCopied = false), 1500);
	}

	async function copyText(text: string): Promise<boolean> {
		try {
			await navigator.clipboard.writeText(text);
			return true;
		} catch {
			return false;
		}
	}

	async function loadJob(): Promise<void> {
		try {
			const next = await service.getJob(jobId);
			if (!sameData(job, next)) job = next;
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
				const result = await service.getJobLogs(jobId, { after_seq: after, limit });
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
			if (job && !notFound) {
				await loadLines();
			}
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
		const timer = setInterval(() => void refresh(), isStreamConnected() ? STREAM_POLL_MS : POLL_MS);
		return () => clearInterval(timer);
	});

	// A step that ends or a log chunk arrives in a burst, so one read serves the burst. A read that
	// is busy runs again after it ends.
	const refreshSoon = coalesce(async () => {
		while (busy) await new Promise((resolve) => setTimeout(resolve, 50));
		await refresh();
	}, EVENT_WAIT_MS);

	onMount(() =>
		onEvent((event) => {
			if (event.type === 'resync' || event.job_id === jobId) refreshSoon();
		})
	);

	async function copyLog(shown: JobLogLine[]): Promise<void> {
		try {
			await navigator.clipboard.writeText(logText(shown));
			pushToast('success', t('jobDetail.logCopied'));
		} catch {
			pushToast('danger', t('jobDetail.logCopyFailed'));
		}
	}

	// The file holds the whole log, not the lines that the view holds.
	async function downloadLog(): Promise<void> {
		if (downloading) return;
		downloading = true;
		try {
			const all: JobLogLine[] = [];
			for (;;) {
				const after = all.length > 0 ? all[all.length - 1].seq : 0;
				const result = await service.getJobLogs(jobId, { after_seq: after, limit: DOWNLOAD_PAGE });
				const fresh = result.items.filter((item) => item.seq > after);
				all.push(...fresh);
				if (!result.has_more || fresh.length === 0) break;
			}
			const url = URL.createObjectURL(
				new Blob([logText(all), '\n'], { type: 'text/plain;charset=utf-8' })
			);
			const link = document.createElement('a');
			link.href = url;
			link.download = `job-${jobId}-log.txt`;
			link.click();
			URL.revokeObjectURL(url);
		} catch {
			pushToast('danger', t('jobDetail.downloadFailed'));
		} finally {
			downloading = false;
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
			const result = await service.cancelJob(jobId);
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

<RecordPage
	title={job?.id ?? jobId}
	description={subtitle}
	crumbs={[{ label: jobId }]}
	{loading}
	error={notFound ? t('jobDetail.notFound') : error && !job ? error : null}
>
	{#snippet status()}
		{#if job}
			<StatusPill
				{tone}
				title={warnings ? t('jobs.statusFull.succeeded_with_warnings') : undefined}
			>
				{statusLabel}
			</StatusPill>
		{/if}
	{/snippet}
	{#snippet actions()}
		{#if job}
			<Button icon="download" loading={downloading} onclick={downloadLog}>
				{t('jobDetail.downloadLog')}
			</Button>
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
		{/if}
	{/snippet}
	{#snippet banners()}
		{#if job}
			{#if jobDetailReason(job)}
				<p class="head__reason" class:head__reason--muted={cancelled}>{jobDetailReason(job)}</p>
			{/if}
			{#if stats.length > 0}
				<Panel title={t('jobDetail.summary')} subtitle={t('jobDetail.summaryHint')}>
					<div class="stats">
						{#each stats as stat (stat.label)}
							<div class="stat">
								<span class="stat__value" class:stat__value--mono={stat.mono}>{statText(stat)}</span
								>
								<span class="stat__label">{t(stat.label)}</span>
							</div>
						{/each}
					</div>
				</Panel>
			{/if}
		{/if}
	{/snippet}
	{#snippet side()}
		{#if job}
			{#if job.restore}
				<RestoreParams restore={job.restore} hostname={job.agent_hostname} />
			{/if}
			<Panel title={t('jobDetail.about')}>
				<dl class="about">
					<dt>{t('jobDetail.id')}</dt>
					<dd class="about__key">
						<span class="mono about__id" title={job.id}>{job.id}</span>
						<Button
							size="sm"
							icon={idCopied ? 'check' : 'copy'}
							aria-label={idCopied ? t('common.copied') : t('jobDetail.id.copy')}
							onclick={() => copyId(job?.id ?? '')}
						/>
					</dd>
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
					<dd class="about__time">{formatLocalTime(job.created_at)}</dd>
					{#if job.started_at}
						<dt>{t('jobDetail.startedAt')}</dt>
						<dd class="about__time">{formatLocalTime(job.started_at)}</dd>
					{/if}
					{#if job.finished_at}
						<dt>{t('jobDetail.finishedAt')}</dt>
						<dd class="about__time">{formatLocalTime(job.finished_at)}</dd>
					{/if}
					{#if job.expires_at && (job.status === 'queued' || job.status === 'sent')}
						<dt>{t('jobDetail.acceptUntil')}</dt>
						<dd class="about__time">{formatLocalTime(job.expires_at)}</dd>
					{/if}
					{#if limitLines(job)}
						<dt>{t('jobDetail.limit')}</dt>
						<dd>
							{#each limitLines(job) ?? [] as line (line)}
								<span class="about__line">{line}</span>
							{/each}
						</dd>
					{/if}
					{#if job.attempt > 0}
						<dt>{t('jobDetail.attempt')}</dt>
						<dd>{job.attempt + 1}</dd>
					{/if}
					{#if job.signing_key_id}
						<dt>{t('jobDetail.signingKey')}</dt>
						<dd class="about__key">
							<span class="mono" title={job.signing_key_id}>{shortKeyId(job.signing_key_id)}</span>
							<Button
								size="sm"
								icon={keyCopied ? 'check' : 'copy'}
								aria-label={keyCopied ? t('common.copied') : t('jobDetail.signingKey.copy')}
								onclick={() => copyKeyId(job?.signing_key_id ?? '')}
							/>
						</dd>
					{/if}
				</dl>
			</Panel>
		{/if}
	{/snippet}

	{#if job}
		<JobSteps {job} {rows} {groups} {progress} {runningProgress} {live} onCopy={copyLog} />

		<div class="gap"></div>
		{#if logPurged}
			<Alert tone="info" title={t('jobDetail.logPurged')}></Alert>
			<div class="gap"></div>
		{/if}
		{#if groups.general.length > 0}
			<div class="gap"></div>
			<Panel title={t('jobDetail.generalLog')} subtitle={t('jobDetail.generalLogHint')}>
				<StepLog
					stepId="general"
					name={t('jobDetail.generalLog')}
					lines={groups.general}
					onCopy={copyLog}
				/>
			</Panel>
		{/if}
		{#if logTruncated}
			<p class="muted note">{t('jobDetail.logTruncated')}</p>
		{/if}
		{#if capped}
			<p class="muted note">{t('jobDetail.logCapped', { count: formatCount(MAX_LINES) })}</p>
		{/if}
		{#if logError}
			<p class="note note--error">{t('jobDetail.logError')}</p>
		{/if}
	{/if}
</RecordPage>

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
	.mono {
		font-family: var(--rh-font-mono);
	}

	.muted {
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.head__reason {
		margin: var(--rh-space-2) 0 0;
		color: var(--rh-color-danger-text);
		font-size: var(--rh-font-size-sm);
	}

	.head__reason--muted {
		color: var(--rh-color-text-muted);
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

	@media (max-width: 1000px) {
		.stats {
			grid-template-columns: repeat(2, minmax(0, 1fr));
		}
	}

	.note {
		margin: var(--rh-space-2) 0 0;
		font-size: var(--rh-font-size-sm);
	}

	.note--error {
		color: var(--rh-color-danger-text);
	}

	.about {
		display: flex;
		flex-direction: column;
		margin: 0;
	}

	.about dt {
		margin: 0;
		padding-top: var(--rh-space-3);
		border-top: var(--rh-border-width) solid var(--rh-color-border-muted);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.about dd {
		margin: 0;
		padding: var(--rh-space-1) 0 var(--rh-space-3);
		font-size: var(--rh-font-size-base);
		overflow-wrap: anywhere;
	}

	.about dt:first-of-type {
		padding-top: 0;
		border-top: 0;
	}

	.about__line {
		display: block;
	}

	.about__time {
		white-space: nowrap;
	}

	.about__key {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--rh-space-2);
	}

	.about__id {
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
</style>
