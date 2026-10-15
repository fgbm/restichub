<script lang="ts">
	import { untrack } from 'svelte';
	import { service } from '#lib/api/index.ts';
	import type { JobLogLine } from '#lib/api/index.ts';
	import { formatClock } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.ts';

	let {
		jobId,
		stepId,
		name,
		tick = 0
	}: {
		jobId: string;
		stepId: string;
		/** The name of the step, for the label of the log. */
		name: string;
		/** Changes whenever the job was read again, so a running step reads its new lines. */
		tick?: number;
	} = $props();

	const PAGE = 500;
	// One step shows this many lines at most. A longer log shows its first lines.
	const MAX_LINES = 5000;

	let lines = $state<JobLogLine[]>([]);
	let loading = $state(true);
	let failed = $state(false);
	let truncated = $state(false);
	let busy = false;

	// Reads the lines after the last held line until the step log ends.
	async function load(): Promise<void> {
		if (busy) return;
		busy = true;
		try {
			while (lines.length < MAX_LINES) {
				const after = lines.length > 0 ? lines[lines.length - 1].seq : 0;
				const page = await service.getJobLogs(jobId, {
					step_id: stepId,
					after_seq: after,
					limit: Math.min(PAGE, MAX_LINES - lines.length)
				});
				failed = false;
				truncated = page.truncated;
				const fresh = page.items.filter((item) => item.seq > after);
				if (fresh.length > 0) lines = [...lines, ...fresh];
				if (!page.has_more || fresh.length === 0) break;
			}
		} catch {
			failed = true;
		} finally {
			busy = false;
			loading = false;
		}
	}

	$effect(() => {
		void tick;
		untrack(() => void load());
	});
</script>

<div class="step-log" data-testid={`step-log-${stepId}`}>
	{#if failed}
		<p class="step-log__note step-log__note--error">{t('jobDetail.stepLogError')}</p>
	{:else if loading}
		<p class="step-log__note">{t('jobDetail.stepLogLoading')}</p>
	{:else if lines.length === 0}
		<p class="step-log__note">{t('jobDetail.stepLogEmpty')}</p>
	{:else}
		<!-- A scrollable region needs a keyboard stop, so a keyboard user can read a long log. -->
		<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
		<div
			class="step-log__body"
			role="log"
			aria-label={`${t('jobDetail.stepLog')}: ${name}`}
			tabindex="0"
		>
			{#each lines as line (line.seq)}
				<div class="step-log__line">
					<span class="step-log__time">{formatClock(line.ts)}</span>
					<span class="step-log__level step-log__level--{line.level}"
						>{t(`jobDetail.level.${line.level}`)}</span
					>
					<span class="step-log__msg">{line.text}</span>
				</div>
			{/each}
		</div>
		{#if truncated}
			<p class="step-log__note">{t('jobDetail.logTruncated')}</p>
		{/if}
	{/if}
</div>

<style>
	.step-log {
		margin-top: var(--rh-space-2);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-code-bg);
	}

	.step-log__note {
		margin: 0;
		padding: var(--rh-space-3);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.step-log__note--error {
		color: var(--rh-color-danger-text);
	}

	.step-log__body {
		max-height: 260px;
		overflow: auto;
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-xs);
		line-height: 1.65;
	}

	.step-log__line {
		display: flex;
		gap: var(--rh-space-3);
		padding: 1px var(--rh-space-3);
		white-space: pre-wrap;
		overflow-wrap: anywhere;
	}

	.step-log__line:hover {
		background-color: var(--rh-color-hover);
	}

	.step-log__time {
		flex: none;
		color: var(--rh-color-text-subtle);
	}

	.step-log__level {
		flex: none;
		width: 44px;
		font-weight: var(--rh-font-weight-semibold);
	}

	.step-log__level--info {
		color: var(--rh-color-info-text);
	}

	.step-log__level--warn {
		color: var(--rh-color-warning-text);
	}

	.step-log__level--error {
		color: var(--rh-color-danger-text);
	}

	.step-log__msg {
		min-width: 0;
		color: var(--rh-color-text);
	}
</style>
