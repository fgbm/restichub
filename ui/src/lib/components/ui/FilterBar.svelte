<script lang="ts">
	import { getContext, type Snippet } from 'svelte';
	import {
		CUSTOM_PERIOD,
		PERIOD_PRESETS,
		hasActiveFilters,
		joinList,
		narrowsPeriod,
		periodOf,
		splitList,
		toggleValue,
		type FilterState
	} from '#lib/filters.ts';
	import { formatCount } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.ts';
	import Button from './Button.svelte';
	import Checkbox from './Checkbox.svelte';
	import DropdownMenu from './DropdownMenu.svelte';
	import Icon from './Icon.svelte';
	import Popover from './Popover.svelte';
	import TextField from './TextField.svelte';
	import Tooltip from './Tooltip.svelte';
	import type { FilterFacet, FilterSearch, FilterSort, FilterTabs, MenuItem } from './types';

	let {
		label,
		value,
		search,
		facets = [],
		period = false,
		states,
		total,
		totalNote,
		sort,
		hint,
		end,
		onchange,
		onreset
	}: {
		/** The accessible name of the bar. */
		label: string;
		/** The current filters; a multi-value facet joins its values with a comma. */
		value: FilterState;
		search?: FilterSearch;
		facets?: FilterFacet[];
		/** Show the period facet. It writes the `period`, `from` and `to` keys. */
		period?: boolean;
		/** The state tabs with counts at the left of the list header. They write the `state` key. */
		states?: FilterTabs;
		/** The count at the left of the list header of an entity without states. */
		total?: number | string;
		/** A short text after the count, for example the period of the count. */
		totalNote?: string;
		/** The Sort menu. It writes the `sort` and `order` keys; empty means the default. */
		sort?: FilterSort;
		/** A short help text behind an info icon. */
		hint?: string;
		/** Content at the right end of the row, for example the age of the list and a refresh button. */
		end?: Snippet;
		onchange: (patch: FilterState) => void;
		/** Default: every key of the bar becomes empty. */
		onreset?: () => void;
	} = $props();

	let opened = $state<string | null>(null);
	let facetQuery = $state('');

	// Inside a list view the bar is the list header: search, then tabs, facets and sort, then chips.
	const inList = getContext<boolean>('rh-list-view') ?? false;
	const statesKey = $derived(states?.key ?? 'state');
	const currentState = $derived(states ? value[statesKey] || states.default : '');
	const sortField = $derived(sort ? value.sort || sort.default : '');
	const sortOption = $derived(sort?.options.find((option) => option.value === sortField));
	const sortOrder = $derived(value.order || sortOption?.order || 'asc');

	function pickState(next: string): void {
		if (!states) return;
		onchange({ [statesKey]: next === states.default ? '' : next });
	}

	function pickSort(next: string): void {
		onchange({ sort: next === sort?.default ? '' : next, order: '' });
	}

	function pickOrder(next: 'asc' | 'desc'): void {
		onchange({ order: next === (sortOption?.order ?? 'asc') ? '' : next });
	}

	const sortItems = $derived<MenuItem[]>(
		sort
			? [
					...sort.options.map((option) => ({
						label: option.label,
						group: 'sort-field',
						groupLabel: t('filter.sortBy'),
						checked: option.value === sortField,
						onSelect: () => pickSort(option.value)
					})),
					...(['asc', 'desc'] as const).map((order, index) => ({
						label: t(order === 'asc' ? 'filter.sortAsc' : 'filter.sortDesc'),
						group: 'sort-order',
						groupLabel: t('filter.sortOrder'),
						separatorBefore: index === 0,
						checked: order === sortOrder,
						onSelect: () => pickOrder(order)
					}))
				]
			: []
	);

	// A long option list gets a search field inside the popover.
	const FACET_SEARCH_FROM = 7;

	function visibleOptions(facet: FilterFacet) {
		const all = optionsOf(facet);
		const query = facetQuery.trim().toLowerCase();
		if (all.length < FACET_SEARCH_FROM || query === '') return all;
		return all.filter((option) => option.label.toLowerCase().includes(query));
	}

	const periodValue = $derived(periodOf(value));

	function isText(facet: FilterFacet): boolean {
		return facet.options === undefined;
	}

	function selected(facet: FilterFacet): string[] {
		if (isText(facet)) return value[facet.key]?.trim() ? [value[facet.key].trim()] : [];
		return facet.multiple === false
			? value[facet.key]
				? [value[facet.key]]
				: []
			: splitList(value[facet.key]);
	}

	// A value from the address that the options do not hold stays in the facet, so a person can clear it.
	function optionsOf(facet: FilterFacet): { value: string; label: string; count?: number }[] {
		const options = facet.options ?? [];
		const known = new Set(options.map((option) => option.value));
		return [
			...options.map((option) => ({ ...option, label: option.label ?? option.value })),
			...selected(facet)
				.filter((item) => !known.has(item))
				.map((item) => ({ value: item, label: item, count: 0 }))
		];
	}

	function labelOf(facet: FilterFacet, item: string): string {
		return facet.options?.find((option) => option.value === item)?.label ?? item;
	}

	function pick(facet: FilterFacet, item: string): void {
		if (facet.multiple === false) {
			onchange({ [facet.key]: value[facet.key] === item ? '' : item });
			opened = null;
		} else {
			onchange({ [facet.key]: joinList(toggleValue(selected(facet), item)) });
		}
	}

	function periodLabel(): string {
		if (periodValue === CUSTOM_PERIOD) {
			return t('filter.periodRange', { from: value.from || '…', to: value.to || '…' });
		}
		return t(`filter.period.${periodValue as (typeof PERIOD_PRESETS)[number]}`);
	}

	function choosePeriod(next: string): void {
		onchange({ period: next, from: '', to: '' });
		if (next !== CUSTOM_PERIOD) opened = null;
	}

	function setOpen(key: string, open: boolean): void {
		if (open) {
			opened = key;
			facetQuery = '';
		} else if (opened === key) opened = null;
	}

	interface Chip {
		key: string;
		text: string;
		remove: () => void;
	}

	const chips = $derived.by<Chip[]>(() => {
		const result: Chip[] = [];
		const text = search ? (value[search.key] ?? '').trim() : '';
		if (search && text !== '') {
			result.push({
				key: search.key,
				text: t('filter.chip', { name: search.label, value: text }),
				remove: () => onchange({ [search.key]: '' })
			});
		}
		for (const facet of facets) {
			for (const item of selected(facet)) {
				result.push({
					key: `${facet.key}:${item}`,
					text: t('filter.chip', { name: facet.label, value: labelOf(facet, item) }),
					remove: () =>
						onchange({
							[facet.key]:
								facet.multiple === false || isText(facet)
									? ''
									: joinList(selected(facet).filter((other) => other !== item))
						})
				});
			}
		}
		if (period && narrowsPeriod(value)) {
			result.push({
				key: 'period',
				text: t('filter.chip', { name: t('filter.period'), value: periodLabel() }),
				remove: () => onchange({ period: '', from: '', to: '' })
			});
		}
		return result;
	});

	const keys = $derived([
		...(search ? [search.key] : []),
		...facets.map((facet) => facet.key),
		...(period ? ['period', 'from', 'to'] : [])
	]);
	const active = $derived(
		hasActiveFilters(Object.fromEntries(keys.map((key) => [key, value[key] ?? ''])))
	);

	function reset(): void {
		if (onreset) onreset();
		else
			onchange(
				Object.fromEntries([...keys, ...(states ? [statesKey] : [])].map((key) => [key, '']))
			);
	}
</script>

{#snippet searchField()}
	{#if search}
		<div class="filter-bar__search">
			<span class="filter-bar__search-icon"><Icon name="search" size="md" /></span>
			<TextField
				id={search.id ?? `filter-${search.key}`}
				type="search"
				mono={search.mono}
				aria-label={search.label}
				placeholder={search.placeholder ?? t('table.filterPlaceholder')}
				value={value[search.key] ?? ''}
				oninput={(event) => onchange({ [search.key]: event.currentTarget.value })}
				class="filter-bar__input"
			/>
		</div>
	{/if}
{/snippet}

{#snippet facetBody(facet: FilterFacet)}
	{@const chosen = selected(facet)}
	{#if facet.hint}
		<p class="facet__hint">{facet.hint}</p>
	{/if}
	{#if isText(facet)}
		<TextField
			id="filter-{facet.key}"
			label={facet.label}
			mono={facet.mono}
			placeholder={facet.placeholder}
			value={value[facet.key] ?? ''}
			oninput={(event) => onchange({ [facet.key]: event.currentTarget.value })}
		/>
	{:else}
		{#if optionsOf(facet).length >= FACET_SEARCH_FROM}
			<div class="facet__search">
				<TextField
					type="search"
					aria-label={t('filter.facetSearch', { name: facet.label })}
					placeholder={t('filter.facetSearch', { name: facet.label })}
					value={facetQuery}
					oninput={(event) => (facetQuery = event.currentTarget.value)}
				/>
			</div>
		{/if}
		<ul class="options">
			{#each visibleOptions(facet) as option (option.value)}
				<li class="option">
					{#if facet.multiple === false}
						<button
							type="button"
							class="choice"
							aria-pressed={chosen.includes(option.value)}
							onclick={() => pick(facet, option.value)}
						>
							<span>{option.label}</span>
							<span class="choice__end">
								{#if option.count !== undefined}
									<span class="option__count">{option.count}</span>
								{/if}
								{#if chosen.includes(option.value)}<Icon name="check" size="md" />{/if}
							</span>
						</button>
					{:else}
						<Checkbox
							label={option.label}
							checked={chosen.includes(option.value)}
							onchange={() => pick(facet, option.value)}
						/>
						{#if option.count !== undefined}
							<span class="option__count">{option.count}</span>
						{/if}
					{/if}
				</li>
			{:else}
				<li class="options__empty">{t('filter.facetEmpty')}</li>
			{/each}
		</ul>
	{/if}
{/snippet}

{#snippet periodBody()}
	<ul class="options">
		{#each [...PERIOD_PRESETS, '', CUSTOM_PERIOD] as preset (preset)}
			<li>
				<button
					type="button"
					class="choice"
					aria-pressed={periodValue === preset}
					onclick={() => choosePeriod(preset)}
				>
					{preset === ''
						? t('filter.period.all')
						: preset === CUSTOM_PERIOD
							? t('filter.period.custom')
							: t(`filter.period.${preset as (typeof PERIOD_PRESETS)[number]}`)}
					{#if periodValue === preset}<Icon name="check" size="md" />{/if}
				</button>
			</li>
		{/each}
	</ul>
	{#if periodValue === CUSTOM_PERIOD}
		<div class="range">
			<TextField
				type="date"
				label={t('filter.periodFrom')}
				value={value.from ?? ''}
				max={value.to || undefined}
				oninput={(event) => onchange({ period: CUSTOM_PERIOD, from: event.currentTarget.value })}
			/>
			<TextField
				type="date"
				label={t('filter.periodTo')}
				value={value.to ?? ''}
				min={value.from || undefined}
				oninput={(event) => onchange({ period: CUSTOM_PERIOD, to: event.currentTarget.value })}
			/>
		</div>
	{/if}
{/snippet}

{#snippet facetPopovers()}
	{#each facets as facet (facet.key)}
		{@const chosen = selected(facet)}
		<Popover
			label={facet.label}
			open={opened === facet.key}
			onOpenChange={(open) => setOpen(facet.key, open)}
		>
			{#snippet trigger()}
				<span class="facet" class:facet--active={chosen.length > 0}>
					{facet.label}
					{#if chosen.length > 0 && facet.multiple !== false && !isText(facet)}
						<span class="facet__count">{chosen.length}</span>
					{/if}
					<Icon name="chevron-down" size="xs" />
				</span>
			{/snippet}
			{@render facetBody(facet)}
		</Popover>
	{/each}

	{#if period}
		<Popover
			label={t('filter.period')}
			open={opened === 'period'}
			onOpenChange={(open) => setOpen('period', open)}
		>
			{#snippet trigger()}
				<span class="facet" class:facet--active={narrowsPeriod(value)}>
					{t('filter.period')}
					<Icon name="chevron-down" size="xs" />
				</span>
			{/snippet}
			{@render periodBody()}
		</Popover>
	{/if}
{/snippet}

{#snippet hintIcon()}
	{#if hint}
		<Tooltip content={hint} ariaLabel={hint}>
			<span class="hint"><Icon name="info" size="md" /></span>
		</Tooltip>
	{/if}
{/snippet}

{#snippet chipList()}
	{#if chips.length > 0}
		<ul class="chips" aria-label={t('filter.applied')}>
			{#each chips as chip (chip.key)}
				<li class="chip">
					<span class="chip__text" title={chip.text}>{chip.text}</span>
					<button
						type="button"
						class="chip__remove"
						aria-label={t('filter.chipRemove', { name: chip.text })}
						onclick={chip.remove}
					>
						<Icon name="x" size="xs" />
					</button>
				</li>
			{/each}
		</ul>
	{/if}
{/snippet}

{#if inList}
	<div class="list-filters" role="group" aria-label={label}>
		{#if search}
			<div class="list-filters__search">{@render searchField()}</div>
		{/if}
		<div class="list-head">
			<div class="list-head__start">
				{#if states}
					<div class="tabs" role="group" aria-label={t('filter.states')}>
						{#each states.options as tab (tab.value)}
							<button
								type="button"
								class="tab"
								aria-pressed={tab.value === currentState}
								onclick={() => pickState(tab.value)}
							>
								{tab.label}
								{#if tab.count !== undefined}
									<span class="count" class:count--danger={tab.danger && tab.count > 0}>
										{formatCount(tab.count)}{tab.capped ? '+' : ''}
									</span>
								{/if}
							</button>
						{/each}
					</div>
				{:else if total !== undefined}
					<span class="total">
						{t('filter.total')} <b>{total}</b>{#if totalNote}&nbsp;{totalNote}{/if}
					</span>
				{/if}
			</div>
			<div class="list-head__end">
				<div class="list-head__facets">{@render facetPopovers()}</div>
				{#if facets.length > 0 || period}
					<div class="list-head__compact">
						<Popover
							label={t('filter.filters')}
							open={opened === '__all'}
							onOpenChange={(open) => setOpen('__all', open)}
						>
							{#snippet trigger()}
								<span class="head-btn">
									<Icon name="filter" size="xs" />
									{t('filter.filters')}
									{#if chips.length > 0}<span class="facet__count">{chips.length}</span>{/if}
								</span>
							{/snippet}
							<div class="compact">
								{#each facets as facet (facet.key)}
									<section class="compact__section">
										<h3 class="compact__heading">{facet.label}</h3>
										{@render facetBody(facet)}
									</section>
								{/each}
								{#if period}
									<section class="compact__section">
										<h3 class="compact__heading">{t('filter.period')}</h3>
										{@render periodBody()}
									</section>
								{/if}
							</div>
						</Popover>
					</div>
				{/if}
				{#if sort}
					<div class="list-head__sort">
						<DropdownMenu items={sortItems} align="end" icon="sort">
							<span class="sort__label">{sortOption?.label ?? ''}</span>
							<Icon name="chevron-down" size="xs" />
						</DropdownMenu>
					</div>
					<div class="list-head__sort-compact">
						<DropdownMenu items={sortItems} align="end" icon="sort" label={t('filter.sort')} />
					</div>
				{/if}
				{@render hintIcon()}
				{#if end}
					<div class="filter-bar__end">{@render end()}</div>
				{/if}
			</div>
		</div>
		{#if chips.length > 0 || active}
			<div class="list-chips">
				{@render chipList()}
				{#if active}
					<button type="button" class="reset-link" onclick={reset}>{t('filter.reset')}</button>
				{/if}
			</div>
		{/if}
	</div>
{:else}
	<div class="filter-bar" role="group" aria-label={label}>
		<div class="filter-bar__row">
			{@render searchField()}
			{@render facetPopovers()}
			{@render hintIcon()}

			{#if active}
				<Button variant="ghost" size="sm" onclick={reset}>{t('filter.reset')}</Button>
			{/if}

			{#if end}
				<div class="filter-bar__end">{@render end()}</div>
			{/if}
		</div>

		{@render chipList()}
	</div>
{/if}

<style>
	.filter-bar {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-2);
		padding: var(--rh-space-3) var(--rh-space-4);
		border-bottom: var(--rh-border-width) solid var(--rh-color-border-muted);
	}

	.filter-bar__row {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.filter-bar__search {
		position: relative;
		flex: 1 1 14rem;
		max-width: 26rem;
	}

	.filter-bar__search-icon {
		position: absolute;
		top: 50%;
		left: var(--rh-space-3);
		z-index: 1;
		display: inline-flex;
		transform: translateY(-50%);
		color: var(--rh-color-text-muted);
		pointer-events: none;
	}

	.filter-bar__search :global(.filter-bar__input) {
		padding-left: var(--rh-space-8);
	}

	.facet {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-1);
		height: var(--rh-control-height);
		padding: 0 var(--rh-space-3);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-input-bg);
		color: var(--rh-color-text);
		white-space: nowrap;
	}

	.facet--active {
		border-color: var(--rh-color-link);
	}

	:global(.rh-popover__trigger:hover) .facet {
		background-color: var(--rh-color-hover);
	}

	:global(.rh-popover__trigger:focus-visible) .facet {
		border-color: var(--rh-color-focus);
		box-shadow: 0 0 0 3px var(--rh-color-focus-ring);
	}

	.facet__count {
		min-width: 18px;
		padding: 0 6px;
		border-radius: var(--rh-radius-pill);
		background-color: var(--rh-color-neutral-bg);
		font-size: var(--rh-font-size-xs);
		font-weight: var(--rh-font-weight-semibold);
		line-height: 18px;
		text-align: center;
	}

	.facet__hint {
		max-width: 16rem;
		margin: 0 0 var(--rh-space-2);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.filter-bar__end {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
		margin-left: auto;
	}

	.hint {
		display: inline-flex;
		color: var(--rh-color-text-muted);
	}

	.options {
		max-height: 18rem;
		margin: 0;
		padding: 0;
		overflow-y: auto;
		list-style: none;
	}

	.option {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--rh-space-3);
		padding: var(--rh-space-1) 0;
	}

	.option__count {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
		font-variant-numeric: tabular-nums;
	}

	.options__empty {
		color: var(--rh-color-text-muted);
	}

	.choice {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--rh-space-3);
		width: 100%;
		padding: var(--rh-space-2);
		border: 0;
		border-radius: var(--rh-radius-md);
		background: none;
		color: inherit;
		font: inherit;
		text-align: left;
		cursor: pointer;
	}

	.choice:hover {
		background-color: var(--rh-color-hover);
	}

	.choice[aria-pressed='true'] {
		font-weight: var(--rh-font-weight-semibold);
	}

	.choice__end {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.range {
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: var(--rh-space-2);
		margin-top: var(--rh-space-3);
		padding-top: var(--rh-space-3);
		border-top: var(--rh-border-width) solid var(--rh-color-border-muted);
	}

	.chips {
		display: flex;
		flex-wrap: wrap;
		gap: var(--rh-space-2);
		margin: 0;
		padding: 0;
		list-style: none;
	}

	.chip {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-1);
		max-width: 100%;
		height: 24px;
		padding: 0 var(--rh-space-1) 0 var(--rh-space-2);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-pill);
		background-color: var(--rh-color-selected);
		font-size: var(--rh-font-size-sm);
	}

	.chip__text {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.chip__remove {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 18px;
		height: 18px;
		padding: 0;
		border: 0;
		border-radius: var(--rh-radius-pill);
		background: none;
		color: var(--rh-color-text-muted);
		cursor: pointer;
	}

	.chip__remove:hover {
		background-color: var(--rh-color-hover);
		color: var(--rh-color-text);
	}
	.list-filters {
		container: list-filters / inline-size;
		display: flex;
		flex-direction: column;
	}

	.list-filters__search {
		margin-bottom: var(--rh-space-3);
	}

	.list-filters__search .filter-bar__search {
		max-width: none;
	}

	.list-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--rh-space-3);
		padding: 6px var(--rh-space-2) 6px var(--rh-space-1);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-bottom: 0;
		border-radius: var(--rh-radius-md) var(--rh-radius-md) 0 0;
		background-color: var(--rh-color-canvas-subtle);
	}

	.list-head__start {
		min-width: 0;
	}

	.list-head__end {
		display: flex;
		flex: none;
		align-items: center;
		gap: 2px;
	}

	.list-head__facets {
		display: flex;
		align-items: center;
		gap: 2px;
	}

	.list-head__compact,
	.list-head__sort-compact {
		display: none;
	}

	.tabs {
		display: flex;
		gap: 2px;
		overflow-x: auto;
		scrollbar-width: none;
	}

	.tab {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		height: 28px;
		padding: 0 10px;
		border: 0;
		border-radius: var(--rh-radius-md);
		background: none;
		color: var(--rh-color-text-muted);
		font: inherit;
		font-size: var(--rh-font-size-sm);
		white-space: nowrap;
		cursor: pointer;
	}

	.tab:hover {
		color: var(--rh-color-text);
	}

	.tab[aria-pressed='true'] {
		background-color: var(--rh-color-canvas);
		box-shadow: inset 0 0 0 1px var(--rh-color-border);
		color: var(--rh-color-text);
		font-weight: var(--rh-font-weight-semibold);
	}

	.tab:focus-visible {
		outline: 2px solid var(--rh-color-focus);
		outline-offset: -2px;
	}

	.count {
		display: inline-block;
		min-width: 20px;
		padding: 0 6px;
		border-radius: var(--rh-radius-pill);
		background-color: var(--rh-color-neutral-bg);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-xs);
		font-weight: var(--rh-font-weight-semibold);
		font-variant-numeric: tabular-nums;
		line-height: 18px;
		text-align: center;
	}

	.count--danger {
		background-color: var(--rh-color-danger-subtle);
		color: var(--rh-color-danger-subtle-text);
	}

	.total {
		padding: 0 var(--rh-space-2);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
		white-space: nowrap;
	}

	.total b {
		color: var(--rh-color-text);
	}

	.list-head .facet,
	.head-btn {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-1);
		height: 28px;
		padding: 0 var(--rh-space-2);
		border: 0;
		border-radius: var(--rh-radius-md);
		background: none;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
		white-space: nowrap;
	}

	.list-head .facet--active {
		color: var(--rh-color-text);
		font-weight: var(--rh-font-weight-semibold);
	}

	:global(.rh-popover__trigger:hover) .head-btn,
	:global(.rh-popover__trigger:hover) .list-head .facet {
		background-color: var(--rh-color-hover);
		color: var(--rh-color-text);
	}

	.list-head__sort :global(.rh-menu__trigger),
	.list-head__sort-compact :global(.rh-menu__trigger) {
		height: 28px;
		padding: 0 var(--rh-space-2);
		border: 0;
		background: none;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
		font-weight: var(--rh-font-weight-normal);
	}

	.list-head__sort :global(.rh-menu__trigger:hover),
	.list-head__sort-compact :global(.rh-menu__trigger:hover) {
		background-color: var(--rh-color-hover);
		color: var(--rh-color-text);
	}

	.sort__label {
		white-space: nowrap;
	}

	.facet__search {
		margin-bottom: var(--rh-space-2);
	}

	.compact {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
		max-height: 70vh;
		overflow-y: auto;
	}

	.compact__heading {
		margin: 0 0 var(--rh-space-1);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-xs);
		font-weight: var(--rh-font-weight-semibold);
	}

	.list-chips {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 6px;
		padding: 6px var(--rh-space-2);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-bottom: 0;
		background-color: var(--rh-color-canvas);
	}

	.list-chips .chips {
		gap: 6px;
	}

	.list-chips .chip {
		background-color: var(--rh-color-canvas-subtle);
		font-size: var(--rh-font-size-xs);
	}

	.reset-link {
		padding: 0 var(--rh-space-1);
		border: 0;
		background: none;
		color: var(--rh-color-link);
		font: inherit;
		font-size: var(--rh-font-size-sm);
		cursor: pointer;
	}

	.reset-link:hover {
		text-decoration: underline;
	}

	@container list-filters (max-width: 640px) {
		.list-head {
			padding: var(--rh-space-1);
		}

		.list-head__start {
			mask-image: linear-gradient(90deg, #000 80%, transparent);
		}

		.list-head__facets,
		.list-head__sort {
			display: none;
		}

		.list-head__compact,
		.list-head__sort-compact {
			display: block;
		}
	}
</style>
