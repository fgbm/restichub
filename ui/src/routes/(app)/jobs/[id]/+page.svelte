<script lang="ts">
	import RestoreParams from '#lib/components/RestoreParams.svelte';
	import { onMount } from 'svelte';
	import { coalesce } from '#lib/events.ts';
	import { isStreamConnected, onEvent } from '#lib/stores/events.svelte.ts';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { JobDetail, JobLogLine, JobStep } from '#lib/api/index.ts';
	import StepLog from '#lib/components/StepLog.svelte';
	import { RecordPage } from '#lib/components/layout/index.ts';
	import {
		Alert,
		Badge,
		Button,
		ConfirmDialog,
		Icon,
		Panel,
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
	import {
		isLive,
		limitLines,
		shortKeyId,
		markerOf,
		statsOf,
		stepProgress,
		stepRows
	} from '#lib/job-detail.ts';
	import type { StepRow } from '#lib/job-detail.ts';
	import { countLines, groupLines, logText } from '#lib/job-log.ts';
	import { jobDetailReason, notStartedReason } from '#lib/job-reason.ts';
	import type { LogCounts } from '#lib/job-log.ts';
	import { plural } from '#lib/profile-form.ts';
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

	const MARKER_ICONS = {
		success: 'check-circle',
		warning: 'alert',
		failed: 'x-circle',
		skipped: 'circle-slash',
		idle: 'clock',
		running: 'loader-circle'
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
	let downloading = $state(false);

	// A step opens its log on a click. A running step and a failed step open by themselves.
	let openSteps = $state<Record<string, boolean>>({});

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

	function reasonOf(current: JobDetail): string | null {
		if (cancelled) return null;
		return jobDetailReason(current);
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

	function stepTitle(stepId: string): string {
		const key = `jobs.kind.${stepId.replaceAll('-', '_')}` as MessageKey;
		const text = t(key);
		return text === key ? stepId : text;
	}

	// A script has the name that its author gave it.
	function rowTitle(row: StepRow): string {
		return row.name || stepTitle(row.id);
	}

	function hasLog(row: StepRow): boolean {
		return row.step !== null || row.state === 'running';
	}

	function stepLines(row: StepRow): JobLogLine[] {
		return groups.byStep.get(row.id) ?? [];
	}

	function countsText(counts: LogCounts): string {
		const parts = [`${formatCount(counts.total)} ${plural('jobDetail.unit.line', counts.total)}`];
		if (counts.warnings > 0) parts.push(`${counts.warnings} ${t('jobDetail.level.warn')}`);
		if (counts.errors > 0) parts.push(`${counts.errors} ${t('jobDetail.level.error')}`);
		return parts.join(' · ');
	}

	function isOpen(row: StepRow): boolean {
		const chosen = openSteps[row.id];
		if (chosen !== undefined) return chosen;
		return row.state === 'running' || (row.step !== null && markerOf(row.step.status) === 'failed');
	}

	function toggle(row: StepRow): void {
		openSteps[row.id] = !isOpen(row);
	}

	function stepError(step: JobStep): string | null {
		if (!step.error_code) return null;
		const reason = notStartedReason(step);
		if (reason) return t('jobDetail.notStartedBecause', { reason });
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
			const next = await service.getJob(id);
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
			if (event.type === 'resync' || event.job_id === id) refreshSoon();
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
				const result = await service.getJobLogs(id, { after_seq: after, limit: DOWNLOAD_PAGE });
				const fresh = result.items.filter((item) => item.seq > after);
				all.push(...fresh);
				if (!result.has_more || fresh.length === 0) break;
			}
			const url = URL.createObjectURL(
				new Blob([logText(all), '\n'], { type: 'text/plain;charset=utf-8' })
			);
			const link = document.createElement('a');
			link.href = url;
			link.download = `job-${id}-log.txt`;
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

<RecordPage
	title={job?.id ?? id}
	description={subtitle}
	crumbs={[{ label: id }]}
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
			{#if reasonOf(job)}
				<p class="head__reason">{reasonOf(job)}</p>
			{/if}
			{#if stats.length > 0}
				<Panel title={t('jobDetail.summary')} subtitle={t('jobDetail.summaryHint')}>
					<div class="stats">
						{#each stats as stat (stat.label)}
							<div class="stat">
								<span class="stat__value" class:stat__value--mono={stat.mono}
									>{statValue(stat)}</span
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
		<Panel
			title={t('jobDetail.steps')}
			subtitle={progress && progress.total > 0
				? t('jobDetail.stepsComplete', { done: progress.complete, total: progress.total })
				: undefined}
		>
			{#if rows.length === 0}
				<p class="muted">{t('jobDetail.stepsEmpty')}</p>
			{:else}
				{#if progress && progress.total > 1}
					<progress
						class="progress progress--steps"
						aria-label={t('jobDetail.steps')}
						max="1"
						value={progress.fraction}
					></progress>
				{/if}
				<ol class="timeline">
					{#each rows as row (row.id)}
						{@const marker =
							row.state === 'running' ? 'running' : row.step ? markerOf(row.step.status) : 'idle'}
						<li class="timeline__item">
							<span class="timeline__marker timeline__marker--{marker}">
								<Icon name={MARKER_ICONS[marker]} size="lg" spin={marker === 'running'} />
							</span>
							{#if hasLog(row)}
								<button
									type="button"
									class="timeline__head timeline__toggle"
									aria-expanded={isOpen(row)}
									aria-label={t(isOpen(row) ? 'jobDetail.hideStepLog' : 'jobDetail.showStepLog', {
										name: rowTitle(row)
									})}
									onclick={() => toggle(row)}
								>
									<Icon name={isOpen(row) ? 'chevron-down' : 'chevron-right'} size="xs" />
									<span class="timeline__title">{rowTitle(row)}</span>
									{#if row.kind === 'script' || row.kind === 'hook'}
										<Badge tone="neutral">{t(`jobDetail.stepKind.${row.kind}` as MessageKey)}</Badge
										>
									{/if}
									{#if row.phase}
										<Badge tone="info">{t(`profiles.hooks.phase.${row.phase}` as MessageKey)}</Badge
										>
									{/if}
									{#if !isOpen(row) && stepLines(row).length > 0}
										<span class="timeline__counts">{countsText(countLines(stepLines(row)))}</span>
									{/if}
									<span class="timeline__meta">{rowMeta(row)}</span>
								</button>
							{:else}
								<div class="timeline__head">
									<span class="timeline__title">{rowTitle(row)}</span>
									{#if row.kind === 'script' || row.kind === 'hook'}
										<Badge tone="neutral">{t(`jobDetail.stepKind.${row.kind}` as MessageKey)}</Badge
										>
									{/if}
									{#if row.phase}
										<Badge tone="info">{t(`profiles.hooks.phase.${row.phase}` as MessageKey)}</Badge
										>
									{/if}
									<span class="timeline__meta">{rowMeta(row)}</span>
								</div>
							{/if}
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
							{#if hasLog(row) && isOpen(row)}
								<StepLog
									stepId={row.id}
									name={rowTitle(row)}
									lines={stepLines(row)}
									live={row.state === 'running'}
									empty={notStartedReason(row.step) ? t('jobDetail.stepLogEmptyReason') : undefined}
									onCopy={copyLog}
								/>
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
		background-color: var(--rh-color-canvas);
		color: var(--rh-color-text-muted);
	}

	.timeline__marker--success {
		color: var(--rh-color-success);
	}

	.timeline__marker--warning {
		color: var(--rh-color-warning);
	}

	.timeline__marker--failed {
		color: var(--rh-color-danger);
	}

	.timeline__marker--running {
		color: var(--rh-color-running);
	}

	.timeline__head {
		display: flex;
		align-items: baseline;
		gap: var(--rh-space-3);
	}

	.timeline__toggle {
		align-items: center;
		width: 100%;
		padding: 0;
		border: 0;
		background: none;
		color: inherit;
		font: inherit;
		text-align: start;
		cursor: pointer;
	}

	.timeline__toggle:hover .timeline__title {
		text-decoration: underline;
	}

	.timeline__title {
		font-weight: var(--rh-font-weight-semibold);
	}

	.timeline__counts {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
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
