<script lang="ts">
	import { tick } from 'svelte';
	import type { JobLogLine } from '#lib/api/index.ts';
	import { Button } from '#lib/components/ui/index.ts';
	import { formatClock } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.ts';
	import { filterLines, scrolledAwayFromEnd } from '#lib/job-log.ts';
	import type { LevelFilter } from '#lib/job-log.ts';

	let {
		stepId,
		name,
		lines,
		live = false,
		onCopy
	}: {
		stepId: string;
		/** The name of the step, for the label of the log. */
		name: string;
		lines: JobLogLine[];
		/** The step is running: the log offers to follow its end. */
		live?: boolean;
		onCopy: (lines: JobLogLine[]) => void;
	} = $props();

	let level = $state<LevelFilter>('all');
	let query = $state('');
	let following = $state(true);
	let body = $state<HTMLDivElement>();

	const shown = $derived(filterLines(lines, level, query));
	const filtered = $derived(level !== 'all' || query.trim() !== '');

	$effect(() => {
		// A new line moves the view to the end while a running step is followed.
		void shown.length;
		if (!live || !following || !body) return;
		void tick().then(() => {
			if (body) body.scrollTop = body.scrollHeight;
		});
	});

	function onScroll(): void {
		// A person who scrolls up stops the following.
		if (following && body && scrolledAwayFromEnd(body)) following = false;
	}
</script>

<div class="step-log" data-testid={`step-log-${stepId}`}>
	{#if lines.length === 0}
		<p class="step-log__note">{t('jobDetail.stepLogEmpty')}</p>
	{:else}
		<div class="step-log__bar">
			<select
				class="rh-select step-log__level-filter"
				aria-label={t('jobDetail.levelFilter')}
				bind:value={level}
			>
				<option value="all">{t('jobDetail.levelFilter.all')}</option>
				<option value="problems">{t('jobDetail.levelFilter.problems')}</option>
			</select>
			<input
				class="rh-input step-log__search"
				type="search"
				aria-label={t('jobDetail.logSearch')}
				placeholder={t('jobDetail.logSearch')}
				bind:value={query}
			/>
			<div class="step-log__actions">
				{#if live}
					<Button size="sm" aria-pressed={following} onclick={() => (following = !following)}>
						{t('jobDetail.follow')}
					</Button>
				{/if}
				<Button size="sm" icon="copy" onclick={() => onCopy(shown)} disabled={shown.length === 0}>
					{t('jobDetail.copyLog')}
				</Button>
			</div>
		</div>
		{#if shown.length === 0}
			<p class="step-log__note">{t('jobDetail.logNoMatch')}</p>
		{:else}
			<!-- A scrollable region needs a keyboard stop, so a keyboard user can read a long log. -->
			<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
			<div
				class="step-log__body"
				role="log"
				aria-label={`${t('jobDetail.stepLog')}: ${name}`}
				tabindex="0"
				bind:this={body}
				onscroll={onScroll}
			>
				{#each shown as line (line.seq)}
					<div class="step-log__line">
						<span class="step-log__no">{line.seq}</span>
						<span class="step-log__time">{formatClock(line.ts)}</span>
						<span class="step-log__level step-log__level--{line.level}"
							>{t(`jobDetail.level.${line.level}`)}</span
						>
						<span class="step-log__msg">{line.text}</span>
					</div>
				{/each}
			</div>
			{#if filtered}
				<p class="step-log__note step-log__note--foot">
					{t('jobDetail.logShown', { shown: shown.length, total: lines.length })}
				</p>
			{/if}
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

	.step-log__bar {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--rh-space-2);
		padding: var(--rh-space-2) var(--rh-space-3);
		border-bottom: var(--rh-border-width) solid var(--rh-color-border);
		background-color: var(--rh-color-surface);
	}

	.step-log__level-filter {
		flex: none;
		width: 220px;
	}

	.step-log__search {
		flex: 1;
		min-width: 120px;
		max-width: 260px;
	}

	.step-log__actions {
		display: flex;
		gap: var(--rh-space-2);
		margin-inline-start: auto;
	}

	.step-log__note--foot {
		border-top: var(--rh-border-width) solid var(--rh-color-border);
		font-size: var(--rh-font-size-xs);
	}

	.step-log__no {
		flex: none;
		min-width: 36px;
		text-align: end;
		color: var(--rh-color-text-subtle);
		user-select: none;
	}

	.step-log__body {
		max-height: 320px;
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
		overflow-wrap: break-word;
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
