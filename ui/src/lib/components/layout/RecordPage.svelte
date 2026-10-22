<script lang="ts">
	import type { Snippet } from 'svelte';
	import { beforeNavigate } from '$app/navigation';
	import { t } from '#lib/i18n/index.ts';
	import type { Crumb } from '#lib/stores/page-meta.svelte.ts';
	import Button from '../ui/Button.svelte';
	import Page from './Page.svelte';
	import type { SaveBar } from './types';

	let {
		title,
		crumbs,
		listQuery,
		description,
		loading = false,
		error = null,
		status,
		actions,
		banners,
		side,
		save,
		unsaved = false,
		onsubmit,
		children
	}: {
		title: string;
		/** The breadcrumb levels below the section. Default: the title as the only level. */
		crumbs?: Crumb[];
		listQuery?: string;
		description?: string;
		loading?: boolean;
		error?: string | null;
		status?: Snippet;
		actions?: Snippet;
		/** Notices about the whole record, above the sections. */
		banners?: Snippet;
		/** The sticky side panel: the summary or the parameters of the record. */
		side?: Snippet;
		/** The save bar of an editor at the bottom of the sections. */
		save?: SaveBar;
		/**
		 * Changes that are not saved outside the save bar, for example a table that saves on its
		 * own. The page asks before it lets them go.
		 */
		unsaved?: boolean;
		/** Makes the sections and the save bar one form. */
		onsubmit?: (event: SubmitEvent) => void;
		/** The sections of the record. */
		children: Snippet;
	} = $props();

	const uid = $props.id();
	const stateId = `record-save-${uid}`;
	let stuck = $state(false);

	// "Cancel" of the save bar drops the changes on purpose, so it leaves without a question.
	let discarding = false;

	// Every editor of a record saves on its own, so the record holds the guard for the changes that
	// are not saved. A save in flight may leave without a question (UI principles 5.5).
	beforeNavigate(({ type, cancel }) => {
		if (discarding || !(save?.dirty || unsaved) || save?.saving) return;
		// A leave shows the dialog of the browser, so the page does not ask a second time.
		if (type === 'leave' || !window.confirm(t('rp.unsaved'))) cancel();
	});

	function discard(): void {
		discarding = true;
		save?.oncancel();
	}

	// The bar casts a shadow only while the sections scroll under it, that is while the end of the
	// sections is below the top edge of the bar.
	function watchEnd(node: HTMLElement): (() => void) | void {
		if (typeof IntersectionObserver === 'undefined') return;
		const bar = node.closest('.record')?.querySelector<HTMLElement>('.record__bar');
		const observer = new IntersectionObserver(([entry]) => (stuck = !entry.isIntersecting), {
			rootMargin: `0px 0px -${bar?.offsetHeight ?? 0}px 0px`
		});
		observer.observe(node);
		return () => observer.disconnect();
	}
</script>

<Page
	{title}
	crumbs={crumbs ?? [{ label: title }]}
	{listQuery}
	{description}
	{loading}
	{error}
	{status}
	{actions}
>
	{#if banners}
		<div class="record__banners">{@render banners()}</div>
	{/if}
	<svelte:element
		this={onsubmit ? 'form' : 'div'}
		class="record"
		class:record--side={Boolean(side)}
		class:record--save={Boolean(save)}
		onsubmit={onsubmit
			? (event: SubmitEvent) => {
					event.preventDefault();
					onsubmit?.(event);
				}
			: undefined}
	>
		<div class="record__main">
			{@render children()}
			{#if save}
				<div class="record__end" {@attach watchEnd}></div>
			{/if}
		</div>
		{#if side}
			<aside class="record__side">{@render side()}</aside>
		{/if}
		{#if save}
			<div class="record__bar" class:record__bar--stuck={stuck}>
				<span class="record__state" id={stateId}>
					<span class="record__status">
						{#if save.readonly}
							{save.readonly}
						{:else if save.dirty}
							<span class="record__dot" aria-hidden="true"></span>
							{t('record.save.dirty')}
						{:else if !save.create}
							{t('record.save.clean')}
						{/if}
					</span>
					{#if save.blocked && !save.readonly}
						<span class="record__blocked">{save.blocked}</span>
					{:else if save.note && !save.readonly}
						<span class="record__note">{save.note}</span>
					{/if}
				</span>
				{#if !save.readonly}
					{#if save.dirty}
						<Button onclick={discard}>{t('common.cancel')}</Button>
					{/if}
					<Button
						type="submit"
						variant="primary"
						loading={save.saving}
						disabled={(!save.dirty && !save.create) || Boolean(save.blocked)}
						title={save.blocked ?? undefined}
						aria-describedby={stateId}
					>
						{t('common.save')}
					</Button>
				{/if}
			</div>
		{/if}
	</svelte:element>
</Page>

<style>
	.record__banners {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
		margin-bottom: var(--rh-space-4);
	}

	.record {
		--record-bar-height: 56px;
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		grid-template-areas:
			'main'
			'bar';
		gap: var(--rh-space-4);
		align-items: start;
	}

	.record--side {
		grid-template-columns: minmax(0, 1fr) var(--rh-layout-side-width);
		grid-template-areas:
			'main side'
			'bar .';
	}

	.record__main {
		grid-area: main;
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
		min-width: 0;
	}

	.record__side {
		grid-area: side;
		position: sticky;
		top: calc(var(--rh-layout-topbar-height) + var(--rh-space-4));
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
		min-width: 0;
		max-height: calc(100vh - var(--rh-layout-topbar-height) - var(--rh-space-8));
		overflow-y: auto;
	}

	.record--save .record__side {
		max-height: calc(
			100vh - var(--rh-layout-topbar-height) - var(--rh-space-8) - var(--record-bar-height)
		);
	}

	.record__side > :global(*) {
		flex-shrink: 0;
	}

	.record__bar {
		position: sticky;
		bottom: 0;
		z-index: var(--rh-z-sticky);
		grid-area: bar;
		display: flex;
		align-items: center;
		gap: var(--rh-space-3);
		min-height: var(--record-bar-height);
		padding: var(--rh-space-2) var(--rh-space-4);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-surface-muted);
	}

	.record__bar--stuck {
		box-shadow: var(--rh-shadow-bar);
	}

	.record__state {
		display: flex;
		flex: 1;
		flex-direction: column;
		gap: 2px;
		min-width: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.record__status {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.record__dot {
		flex: none;
		width: 8px;
		height: 8px;
		border-radius: var(--rh-radius-pill);
		background-color: var(--rh-color-warning);
	}

	.record__note {
		font-size: var(--rh-font-size-xs);
	}

	.record__blocked {
		color: var(--rh-color-danger-text);
		font-size: var(--rh-font-size-xs);
	}

	.record__end {
		height: 1px;
		margin-top: calc(-1 * var(--rh-space-3) - 1px);
	}

	@media (max-width: 1023px) {
		.record--side {
			grid-template-columns: minmax(0, 1fr);
			grid-template-areas:
				'side'
				'main'
				'bar';
		}

		.record__side,
		.record--save .record__side {
			position: static;
			max-height: none;
		}
	}
</style>
