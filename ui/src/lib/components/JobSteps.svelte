<script lang="ts">
	import type { JobDetail, JobLogLine, JobStep } from '#lib/api/index.ts';
	import { Badge, Icon, Panel } from '#lib/components/ui/index.ts';
	import { formatBytes, formatCount, formatDuration, formatSeconds } from '#lib/format.ts';
	import {
		isInitProbe,
		markerOf,
		statText,
		statsOf,
		stepErrorCode,
		type Stat,
		type StepMarker,
		type StepProgressSummary,
		type StepRow
	} from '#lib/job-detail.ts';
	import type { LogCounts, LogGroups } from '#lib/job-log.ts';
	import { countLines } from '#lib/job-log.ts';
	import { notStartedReason } from '#lib/job-reason.ts';
	import { t } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { plural } from '#lib/profile-form.ts';
	import StepLog from './StepLog.svelte';

	const MARKER_ICONS = {
		success: 'check-circle',
		warning: 'alert',
		failed: 'x-circle',
		skipped: 'circle-slash',
		idle: 'clock',
		running: 'loader-circle',
		info: 'info'
	} as const;

	const {
		job,
		rows,
		groups,
		progress,
		runningProgress,
		live,
		onCopy
	}: {
		job: JobDetail;
		rows: StepRow[];
		groups: LogGroups;
		progress: StepProgressSummary | null;
		runningProgress: JobDetail['progress'];
		live: boolean;
		onCopy: (lines: JobLogLine[]) => void;
	} = $props();

	// A step opens its log on a click. A running step and a failed step open by themselves.
	let openSteps = $state<Record<string, boolean>>({});

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

	function markerOfRow(row: StepRow): StepMarker {
		if (row.state === 'running') return 'running';
		if (!row.step) return 'idle';
		if (isInitProbe(job, row.step)) return 'info';
		return markerOf(row.step.status);
	}

	function isOpen(row: StepRow): boolean {
		const chosen = openSteps[row.id];
		if (chosen !== undefined) return chosen;
		return row.state === 'running' || markerOfRow(row) === 'failed';
	}

	function toggle(row: StepRow): void {
		openSteps[row.id] = !isOpen(row);
	}

	function stepError(step: JobStep): string | null {
		if (isInitProbe(job, step)) return t('jobDetail.probe.willCreate');
		const code = stepErrorCode(step);
		if (!code) return null;
		const reason = notStartedReason(step);
		if (reason) return t('jobDetail.notStartedBecause', { reason });
		const key = `jobDetail.stepError.${code}` as MessageKey;
		const text = t(key);
		return text === key ? code : text;
	}

	// The duration is the own time of the step: its start and end come from the host.
	function stepMeta(step: JobStep): string {
		const parts = [
			isInitProbe(job, step)
				? t('jobDetail.probe.absent')
				: t(`jobDetail.stepStatus.${step.status}` as MessageKey)
		];
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

	function statValue(stat: Stat): string {
		return statText(stat);
	}
</script>

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
				{@const marker = markerOfRow(row)}
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
								<Badge tone="neutral">{t(`jobDetail.stepKind.${row.kind}` as MessageKey)}</Badge>
							{/if}
							{#if row.phase}
								<Badge tone="info">{t(`profiles.hooks.phase.${row.phase}` as MessageKey)}</Badge>
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
								<Badge tone="neutral">{t(`jobDetail.stepKind.${row.kind}` as MessageKey)}</Badge>
							{/if}
							{#if row.phase}
								<Badge tone="info">{t(`profiles.hooks.phase.${row.phase}` as MessageKey)}</Badge>
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
							{onCopy}
						/>
					{/if}
				</li>
			{/each}
		</ol>
	{/if}
</Panel>

<style>
	.muted {
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
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

	.timeline__marker--info {
		color: var(--rh-color-info);
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
</style>
