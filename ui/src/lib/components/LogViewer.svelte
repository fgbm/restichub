<script lang="ts">
	import { tick } from 'svelte';
	import type { JobLogLine } from '#lib/api/index.ts';
	import { Button } from '#lib/components/ui/index.ts';
	import { formatClock } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.ts';

	let {
		title,
		lines,
		live = false,
		following = $bindable(false),
		emptyText,
		onCopy
	}: {
		title: string;
		lines: JobLogLine[];
		/** A running job: the viewer offers to follow the end of the log. */
		live?: boolean;
		following?: boolean;
		emptyText: string;
		onCopy: () => void;
	} = $props();

	let body = $state<HTMLDivElement>();
	// The distance from the end below which the viewer counts as scrolled to the end.
	const END_MARGIN = 24;

	$effect(() => {
		// A new line moves the view to the end while a running job is followed.
		void lines.length;
		if (!live || !following || !body) return;
		void tick().then(() => {
			if (body) body.scrollTop = body.scrollHeight;
		});
	});

	function onScroll(): void {
		if (!following || !body) return;
		// A person who scrolls up stops the following.
		if (body.scrollHeight - body.scrollTop - body.clientHeight > END_MARGIN) following = false;
	}
</script>

<section class="log">
	<div class="log__head">
		<h2 class="log__title">{title}</h2>
		<div class="log__actions">
			{#if live && following}
				<span class="log__following"
					><span class="log__spinner" aria-hidden="true"></span>{t('jobDetail.following')}</span
				>
			{/if}
			{#if live}
				<Button size="sm" onclick={() => (following = !following)}>
					{following ? t('jobDetail.pause') : t('jobDetail.follow')}
				</Button>
			{/if}
			<Button size="sm" icon="copy" onclick={onCopy} disabled={lines.length === 0}>
				{t('jobDetail.copyLog')}
			</Button>
		</div>
	</div>
	{#if lines.length === 0}
		<p class="log__empty">{emptyText}</p>
	{:else}
		<!-- A scrollable region needs a keyboard stop, so a keyboard user can read a long log. -->
		<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
		<div
			class="log__body"
			role="log"
			aria-label={t('jobDetail.logLabel')}
			tabindex="0"
			bind:this={body}
			onscroll={onScroll}
		>
			{#each lines as line (line.seq)}
				<div class="log__line">
					<span class="log__no">{line.seq}</span>
					<span class="log__time">{formatClock(line.ts)}</span>
					<span class="log__level log__level--{line.level}"
						>{t(`jobDetail.level.${line.level}`)}</span
					>
					<span class="log__msg">{line.text}</span>
				</div>
			{/each}
		</div>
	{/if}
</section>

<style>
	.log {
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-code-bg);
		overflow: hidden;
	}

	.log__head {
		display: flex;
		align-items: center;
		gap: var(--rh-space-3);
		padding: var(--rh-space-3) var(--rh-space-4);
		border-bottom: var(--rh-border-width) solid var(--rh-color-border);
		background-color: var(--rh-color-surface);
	}

	.log__title {
		margin: 0;
		font-size: var(--rh-font-size-md);
		font-weight: var(--rh-font-weight-semibold);
	}

	.log__actions {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
		margin-inline-start: auto;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.log__following {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.log__spinner {
		width: 12px;
		height: 12px;
		border: 2px solid var(--rh-color-border);
		border-top-color: var(--rh-color-text-muted);
		border-radius: 50%;
		animation: log-spin 0.8s linear infinite;
	}

	@keyframes log-spin {
		to {
			transform: rotate(360deg);
		}
	}

	@media (prefers-reduced-motion: reduce) {
		.log__spinner {
			animation: none;
		}
	}

	.log__empty {
		margin: 0;
		padding: var(--rh-space-5) var(--rh-space-4);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.log__body {
		max-height: 420px;
		overflow: auto;
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-xs);
		line-height: 1.65;
	}

	.log__line {
		display: flex;
		gap: var(--rh-space-3);
		padding: 1px var(--rh-space-3);
		white-space: pre-wrap;
		overflow-wrap: anywhere;
	}

	.log__line:hover {
		background-color: var(--rh-color-hover);
	}

	.log__no {
		flex: none;
		width: 48px;
		text-align: end;
		color: var(--rh-color-text-subtle);
		user-select: none;
	}

	.log__time {
		flex: none;
		color: var(--rh-color-text-subtle);
	}

	.log__level {
		flex: none;
		width: 44px;
		font-weight: var(--rh-font-weight-semibold);
	}

	.log__level--info {
		color: var(--rh-color-info-text);
	}

	.log__level--warn {
		color: var(--rh-color-warning-text);
	}

	.log__level--error {
		color: var(--rh-color-danger-text);
	}

	.log__msg {
		min-width: 0;
		color: var(--rh-color-text);
	}
</style>
