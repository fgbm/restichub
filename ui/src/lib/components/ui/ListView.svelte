<script lang="ts" generics="T">
	import { setContext, type Snippet } from 'svelte';
	import { fullTime } from '#lib/list-format.ts';
	import { t } from '#lib/i18n/index.ts';
	import Button from './Button.svelte';
	import EmptyState from './EmptyState.svelte';
	import Icon from './Icon.svelte';
	import type { IconName } from './icons';
	import ListFooter from './ListFooter.svelte';
	import RowActions from './RowActions.svelte';
	import Skeleton from './Skeleton.svelte';
	import StatusPill from './StatusPill.svelte';
	import type {
		ListField,
		ListIcon,
		ListMeta,
		ListPill,
		ListTone,
		MenuItem,
		RowAction,
		StatusTone
	} from './types';

	let {
		items = [],
		itemKey,
		label,
		icon,
		title,
		titleMono = false,
		titleLabel,
		href,
		onOpen,
		pill,
		meta,
		aside,
		actions,
		moreLabel,
		groupBy,
		mode = 'client',
		page = $bindable(1),
		pageSize = $bindable(25),
		pageSizes = [10, 25, 50, 100],
		paginated = true,
		total,
		totalCapped = false,
		onPageChange,
		onPageSizeChange,
		loading = false,
		error = null,
		filtered = false,
		onReset,
		emptyTitle,
		emptyDescription,
		emptyIcon,
		emptyActions,
		flush = false,
		toolbar
	}: {
		items?: T[];
		itemKey: (item: T) => string;
		/** The accessible name of the list. */
		label: string;
		icon: (item: T) => ListIcon;
		title: (item: T) => string;
		titleMono?: boolean;
		/** The accessible name of the title, when the visible title is too terse. */
		titleLabel?: (item: T) => string;
		/** The resolved address of the record page. The title is the only link of the item. */
		href?: (item: T) => string | undefined;
		/** For an entity without a record page: the title opens a side panel. */
		onOpen?: (item: T) => void;
		/** Only an unusual state has a pill. */
		pill?: (item: T) => ListPill | null;
		/** The fields of line 2 in the order of priority. A narrow list shows the first two. */
		meta?: (item: T) => ListMeta[];
		/** A number in a right-aligned column, such as a duration or a size. */
		aside?: (item: T) => string;
		/** One or two actions show as icon buttons, three or more as one "…" menu. */
		actions?: (item: T) => RowAction[];
		moreLabel?: (item: T) => string;
		/** The heading of the group of the item, for example the day. Sort the items by it. */
		groupBy?: (item: T) => string;
		/** `server`: the items are one page that the server filtered, sorted and cut. */
		mode?: 'client' | 'server';
		page?: number;
		pageSize?: number;
		pageSizes?: number[];
		/** Client mode only. Set to `false` to show all items, for example in a short preview. */
		paginated?: boolean;
		/** Server mode: the number of items that match the filters on all pages. */
		total?: number;
		totalCapped?: boolean;
		onPageChange?: (page: number) => void;
		onPageSizeChange?: (pageSize: number) => void;
		loading?: boolean;
		error?: string | null;
		/** A filter applies: an empty list means "no matches", not "no records yet". */
		filtered?: boolean;
		onReset?: () => void;
		emptyTitle?: string;
		emptyDescription?: string;
		emptyIcon?: IconName;
		/** The primary action of the first-run empty state. */
		emptyActions?: Snippet;
		/** For a list inside a panel body: no outer border. */
		flush?: boolean;
		/** The filter bar above the list. */
		toolbar?: Snippet;
	} = $props();

	// The filter bar draws the list header when it is inside a list view.
	setContext('rh-list-view', true);

	const server = $derived(mode === 'server');
	const totalItems = $derived(server ? (total ?? items.length) : items.length);
	const pageCount = $derived(Math.max(1, Math.ceil(totalItems / pageSize)));
	const currentPage = $derived(Math.min(Math.max(page, 1), pageCount));
	const shown = $derived(
		server || !paginated ? items : items.slice((currentPage - 1) * pageSize, currentPage * pageSize)
	);

	interface Row {
		key: string;
		item: T;
		group: string | null;
	}

	const rows = $derived.by<Row[]>(() => {
		let previous: string | null = null;
		return shown.map((item) => {
			const group = groupBy ? groupBy(item) : null;
			const heading = group !== null && group !== previous ? group : null;
			previous = group;
			return { key: itemKey(item), item, group: heading };
		});
	});

	const SEPARATOR = ' · ';

	const PILL_TONES: Record<ListTone, StatusTone> = {
		danger: 'failed',
		warning: 'warning',
		running: 'running',
		neutral: 'neutral'
	};

	function fieldsOf(item: T): ListField[] {
		return (meta?.(item) ?? [])
			.filter((field): field is ListField | string => Boolean(field))
			.map((field) => (typeof field === 'string' ? { text: field } : field))
			.filter((field) => field.text !== '');
	}

	function menuItems(item: T): MenuItem[] {
		return (actions?.(item) ?? [])
			.filter((action) => !action.hidden)
			.map((action) => ({
				label: action.label,
				hint: action.hint,
				icon: action.icon,
				onSelect: action.onSelect,
				disabled: action.disabled,
				tone: action.danger ? 'danger' : 'default'
			}));
	}

	function goTo(next: number): void {
		page = next;
		onPageChange?.(next);
	}

	function setPageSize(next: number): void {
		pageSize = next;
		page = 1;
		onPageSizeChange?.(next);
	}
</script>

<div class="list-view" class:list-view--flush={flush}>
	{@render toolbar?.()}

	<div class="list-view__box" class:list-view__box--toolbar={Boolean(toolbar)}>
		{#if loading}
			<ul class="list" aria-label={t('table.loading')} aria-busy="true">
				{#each Array(Math.min(pageSize, 8)) as _, index (index)}
					<li class="item item--skeleton">
						<span class="item__lead"><Skeleton variant="avatar" /></span>
						<span class="item__body">
							<Skeleton />
							<Skeleton />
						</span>
					</li>
				{/each}
			</ul>
		{:else if error}
			<p class="list-view__error" role="alert">{error}</p>
		{:else if items.length === 0 && filtered}
			<EmptyState
				icon="search"
				title={t('list.noMatches')}
				description={t('list.noMatchesDescription')}
			>
				{#if onReset}
					<Button onclick={onReset}>{t('list.resetFilters')}</Button>
				{/if}
			</EmptyState>
		{:else if items.length === 0}
			<EmptyState
				icon={emptyIcon}
				title={emptyTitle ?? t('table.empty.title')}
				description={emptyDescription}
			>
				{#if emptyActions}{@render emptyActions()}{/if}
			</EmptyState>
		{:else}
			<ul class="list" aria-label={label}>
				{#each rows as row (row.key)}
					{@const item = row.item}
					{@const lead = icon(item)}
					{@const state = pill?.(item) ?? null}
					{@const fields = fieldsOf(item)}
					{@const number = aside?.(item) ?? ''}
					{@const name = title(item)}
					{@const link = href?.(item)}
					{#if row.group !== null}
						<li class="group">{row.group}</li>
					{/if}
					<li class="item" class:item--aside={Boolean(aside)}>
						<span
							class="item__lead"
							class:tone--danger={lead.tone === 'danger'}
							class:tone--warning={lead.tone === 'warning'}
							class:tone--running={lead.tone === 'running'}
							title={lead.label}
						>
							<Icon name={lead.name} spin={lead.spin} label={state ? undefined : lead.label} />
						</span>
						<span class="item__body">
							<span class="item__line">
								{#if link}
									<!-- eslint-disable svelte/no-navigation-without-resolve -- href returns a resolved path -->
									<a
										class="item__title"
										class:item__title--mono={titleMono}
										href={link}
										title={name}
										aria-label={titleLabel?.(item)}>{name}</a
									>
									<!-- eslint-enable svelte/no-navigation-without-resolve -->
								{:else if onOpen}
									<button
										type="button"
										class="item__title"
										class:item__title--mono={titleMono}
										title={name}
										aria-label={titleLabel?.(item)}
										onclick={() => onOpen?.(item)}>{name}</button
									>
								{:else}
									<span
										class="item__title item__title--text"
										class:item__title--mono={titleMono}
										title={name}>{name}</span
									>
								{/if}
								{#if state}
									<StatusPill tone={PILL_TONES[state.tone]} title={state.full}>
										{state.label}
									</StatusPill>
								{/if}
							</span>
							{#if fields.length > 0 || number}
								<span class="item__meta">
									{#each fields as field, index (index)}
										<span class="item__field" class:item__field--extra={index >= 2}>
											{#if index > 0}<span class="item__sep" aria-hidden="true">{SEPARATOR}</span
												>{/if}
											{#if field.time}
												<time datetime={field.time} title={field.title ?? fullTime(field.time)}
													>{field.text}</time
												>
											{:else}
												<span class:item__mono={field.mono} title={field.title}>{field.text}</span>
											{/if}
										</span>
									{/each}
									{#if number}
										<span class="item__field item__field--aside">
											{#if fields.length > 0}<span class="item__sep" aria-hidden="true"
													>{SEPARATOR}</span
												>{/if}
											<span class="item__number">{number}</span>
										</span>
									{/if}
								</span>
							{/if}
						</span>
						{#if aside}
							<span class="item__aside" aria-hidden="true">{number}</span>
						{/if}
						<span class="item__actions">
							{#if actions}
								<RowActions
									variant="ghost"
									items={menuItems(item)}
									moreLabel={moreLabel?.(item) ?? t('table.moreActions')}
								/>
							{/if}
						</span>
					</li>
				{/each}
			</ul>
		{/if}

		{#if (server || paginated) && !loading && !error && totalItems > 0}
			<ListFooter
				page={currentPage}
				{pageCount}
				{pageSize}
				{pageSizes}
				total={totalItems}
				{totalCapped}
				sizes={!server || Boolean(onPageSizeChange)}
				onPage={goTo}
				onPageSize={setPageSize}
			/>
		{/if}
	</div>
</div>

<style>
	.list-view {
		container: list-view / inline-size;
		min-width: 0;
	}

	.list-view__box {
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-canvas);
		overflow: hidden;
	}

	.list-view__box--toolbar {
		border-top-left-radius: 0;
		border-top-right-radius: 0;
	}

	.list-view--flush .list-view__box {
		border: 0;
		border-radius: 0;
	}

	.list {
		margin: 0;
		padding: 0;
		list-style: none;
	}

	.item {
		display: grid;
		grid-template-columns: 24px minmax(0, 1fr) auto;
		align-items: center;
		column-gap: var(--rh-space-2);
		min-height: 56px;
		padding: var(--rh-space-2) var(--rh-space-2) var(--rh-space-2) var(--rh-space-3);
		border-top: var(--rh-border-width) solid var(--rh-color-border-muted);
	}

	.item--aside {
		grid-template-columns: 24px minmax(0, 1fr) auto auto;
	}

	.list > :first-child {
		border-top: 0;
	}

	.item:hover {
		background-color: var(--rh-color-hover);
	}

	.item__lead {
		display: grid;
		place-items: center;
		align-self: start;
		height: 20px;
		color: var(--rh-color-text-subtle);
	}

	.tone--danger {
		color: var(--rh-color-danger);
	}

	.tone--warning {
		color: var(--rh-color-warning);
	}

	.tone--running {
		color: var(--rh-color-running);
	}

	.item__body {
		display: grid;
		gap: 2px;
		min-width: 0;
	}

	.item__line {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
		min-width: 0;
		line-height: 20px;
	}

	.item__title {
		min-width: 0;
		overflow: hidden;
		padding: 0;
		border: 0;
		background: none;
		color: var(--rh-color-text);
		font: inherit;
		font-weight: var(--rh-font-weight-semibold);
		text-align: left;
		text-decoration: none;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	a.item__title:hover,
	button.item__title:hover {
		color: var(--rh-color-link);
		text-decoration: underline;
		cursor: pointer;
	}

	.item__title:focus-visible {
		outline: 2px solid var(--rh-color-focus);
		outline-offset: 2px;
		border-radius: var(--rh-radius-sm);
	}

	.item__title--mono {
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-sm);
	}

	.item__meta {
		overflow: hidden;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
		line-height: 18px;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.item__sep {
		color: var(--rh-color-text-subtle);
	}

	.item__mono {
		font-family: var(--rh-font-mono);
		font-size: 0.92em;
	}

	.item__meta time,
	.item__number {
		font-variant-numeric: tabular-nums;
		white-space: nowrap;
	}

	.item__field--aside {
		display: none;
	}

	.item__aside {
		width: 96px;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
		font-variant-numeric: tabular-nums;
		text-align: right;
		white-space: nowrap;
	}

	.item__actions {
		display: flex;
		justify-content: flex-end;
		gap: 2px;
		min-width: 28px;
	}

	.group {
		position: sticky;
		top: 0;
		padding: 6px var(--rh-space-3);
		border-top: var(--rh-border-width) solid var(--rh-color-border-muted);
		background-color: var(--rh-color-canvas-subtle);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-xs);
		font-weight: var(--rh-font-weight-semibold);
	}

	.item--skeleton .item__body {
		gap: var(--rh-space-2);
	}

	.item--skeleton .item__body :global(.skeleton:last-child) {
		width: 60%;
	}

	.list-view__error {
		margin: 0;
		padding: var(--rh-space-6) var(--rh-space-4);
		color: var(--rh-color-danger);
		text-align: center;
	}

	/* A narrow list keeps the first two fields of line 2; the number moves to its end. */
	@container list-view (max-width: 560px) {
		.item {
			padding: var(--rh-space-2) var(--rh-space-1) var(--rh-space-2) var(--rh-space-2);
			column-gap: 6px;
		}

		.item--aside {
			grid-template-columns: 24px minmax(0, 1fr) auto;
		}

		.item__field--extra,
		.item__aside {
			display: none;
		}

		.item__field--aside {
			display: inline;
		}
	}
</style>
