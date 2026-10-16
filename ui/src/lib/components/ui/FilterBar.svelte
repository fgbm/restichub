<script lang="ts">
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
	import { t } from '#lib/i18n/index.ts';
	import Button from './Button.svelte';
	import Checkbox from './Checkbox.svelte';
	import Icon from './Icon.svelte';
	import Popover from './Popover.svelte';
	import TextField from './TextField.svelte';
	import Tooltip from './Tooltip.svelte';
	import type { FilterFacet, FilterSearch } from './types';

	let {
		label,
		value,
		search,
		facets = [],
		period = false,
		hint,
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
		/** A short help text behind an info icon. */
		hint?: string;
		onchange: (patch: FilterState) => void;
		/** Default: every key of the bar becomes empty. */
		onreset?: () => void;
	} = $props();

	let opened = $state<string | null>(null);

	const periodValue = $derived(periodOf(value));

	function selected(facet: FilterFacet): string[] {
		return facet.multiple === false
			? value[facet.key]
				? [value[facet.key]]
				: []
			: splitList(value[facet.key]);
	}

	// A value from the address that the options do not hold stays in the facet, so a person can clear it.
	function optionsOf(facet: FilterFacet): { value: string; label: string; count?: number }[] {
		const known = new Set(facet.options.map((option) => option.value));
		return [
			...facet.options.map((option) => ({ ...option, label: option.label ?? option.value })),
			...selected(facet)
				.filter((item) => !known.has(item))
				.map((item) => ({ value: item, label: item, count: 0 }))
		];
	}

	function labelOf(facet: FilterFacet, item: string): string {
		return facet.options.find((option) => option.value === item)?.label ?? item;
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
		if (open) opened = key;
		else if (opened === key) opened = null;
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
								facet.multiple === false
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
		else onchange(Object.fromEntries(keys.map((key) => [key, ''])));
	}
</script>

<div class="filter-bar" role="group" aria-label={label}>
	<div class="filter-bar__row">
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
						{#if chosen.length > 0 && facet.multiple !== false}
							<span class="facet__count">{chosen.length}</span>
						{/if}
						<Icon name="chevron-down" size="xs" />
					</span>
				{/snippet}
				<ul class="options">
					{#each optionsOf(facet) as option (option.value)}
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
							oninput={(event) =>
								onchange({ period: CUSTOM_PERIOD, from: event.currentTarget.value })}
						/>
						<TextField
							type="date"
							label={t('filter.periodTo')}
							value={value.to ?? ''}
							min={value.from || undefined}
							oninput={(event) =>
								onchange({ period: CUSTOM_PERIOD, to: event.currentTarget.value })}
						/>
					</div>
				{/if}
			</Popover>
		{/if}

		{#if hint}
			<Tooltip content={hint} ariaLabel={hint}>
				<span class="hint"><Icon name="info" size="md" /></span>
			</Tooltip>
		{/if}

		{#if active}
			<Button variant="ghost" size="sm" onclick={reset}>{t('filter.reset')}</Button>
		{/if}
	</div>

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
</div>

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
</style>
