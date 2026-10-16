<script lang="ts">
	import { Button, Checkbox, Icon, Popover, TextField, Tooltip } from '#lib/components/ui/index.ts';
	import { t, tOr } from '#lib/i18n/index.ts';
	import type { SnapshotItem } from '#lib/api/index.ts';
	import type { RestoreFilters } from '#lib/restore.ts';
	import {
		CUSTOM_PERIOD,
		PERIOD_PRESETS,
		facetCounts,
		hasFilters,
		joinList,
		narrowsPeriod,
		periodOf,
		splitList,
		tagLabel
	} from '#lib/snapshot-filter.ts';

	let {
		snapshots,
		filters,
		profiles = new Map<string, string>(),
		onchange,
		onreset
	}: {
		/** The whole loaded list: the facets offer the values that it holds. */
		snapshots: readonly SnapshotItem[];
		filters: RestoreFilters;
		/** The names of the profiles by identifier, for the automatic profile tags. */
		profiles?: ReadonlyMap<string, string>;
		onchange: (patch: Partial<RestoreFilters>) => void;
		onreset: () => void;
	} = $props();

	let hostOpen = $state(false);
	let tagOpen = $state(false);
	let periodOpen = $state(false);

	const hosts = $derived(splitList(filters.host));
	const tags = $derived(splitList(filters.tag));
	const period = $derived(periodOf(filters));

	// A value from the address that the list does not hold stays in the facet, so a person can clear it.
	function withSelected(
		counted: { value: string; count: number }[],
		selected: string[]
	): { value: string; count: number }[] {
		const known = new Set(counted.map((item) => item.value));
		return [
			...counted,
			...selected.filter((value) => !known.has(value)).map((value) => ({ value, count: 0 }))
		];
	}

	const hostOptions = $derived(
		withSelected(
			facetCounts(snapshots, (item) => [item.hostname]),
			hosts
		)
	);
	const tagOptions = $derived(
		withSelected(
			facetCounts(snapshots, (item) => item.tags),
			tags
		)
	);

	function toggle(list: string[], value: string): string[] {
		return list.includes(value) ? list.filter((item) => item !== value) : [...list, value];
	}

	function periodLabel(): string {
		if (period === CUSTOM_PERIOD) {
			return t('restore.periodRange', { from: filters.from || '…', to: filters.to || '…' });
		}
		return tOr(`restore.period.${period}`, period);
	}

	function choosePeriod(next: string): void {
		onchange({ period: next, from: '', to: '' });
		if (next !== CUSTOM_PERIOD) periodOpen = false;
	}

	const chips = $derived([
		...hosts.map((value) => ({
			key: `host:${value}`,
			text: t('restore.chipHost', { value }),
			name: t('restore.facetHost'),
			remove: () => onchange({ host: joinList(hosts.filter((item) => item !== value)) })
		})),
		...tags.map((value) => ({
			key: `tag:${value}`,
			text: t('restore.chipTag', { value: tagLabel(value, profiles) }),
			name: t('restore.facetTag'),
			remove: () => onchange({ tag: joinList(tags.filter((item) => item !== value)) })
		})),
		...(filters.path.trim() === ''
			? []
			: [
					{
						key: 'path',
						text: t('restore.chipPath', { value: filters.path.trim() }),
						name: t('restore.pathSearch'),
						remove: () => onchange({ path: '' })
					}
				]),
		...(!narrowsPeriod(filters)
			? []
			: [
					{
						key: 'period',
						text: t('restore.chipPeriod', { value: periodLabel() }),
						name: t('restore.facetPeriod'),
						remove: () => onchange({ period: '', from: '', to: '' })
					}
				])
	]);
</script>

<div class="bar" role="group" aria-label={t('restore.filterBar')}>
	<div class="bar__row">
		<div class="bar__search">
			<span class="bar__search-icon"><Icon name="search" size="md" /></span>
			<TextField
				id="restore-path"
				type="search"
				mono
				aria-label={t('restore.pathSearch')}
				placeholder={t('restore.pathPlaceholder')}
				value={filters.path}
				oninput={(event) => onchange({ path: event.currentTarget.value })}
				class="bar__input"
			/>
		</div>

		<Popover label={t('restore.facetHost')} bind:open={hostOpen}>
			{#snippet trigger()}
				<span class="facet" class:facet--active={hosts.length > 0}>
					{t('restore.facetHost')}
					{#if hosts.length > 0}<span class="facet__count">{hosts.length}</span>{/if}
					<Icon name="chevron-down" size="xs" />
				</span>
			{/snippet}
			<ul class="options">
				{#each hostOptions as option (option.value)}
					<li class="option">
						<Checkbox
							label={option.value}
							checked={hosts.includes(option.value)}
							onchange={() => onchange({ host: joinList(toggle(hosts, option.value)) })}
						/>
						<span class="option__count">{option.count}</span>
					</li>
				{:else}
					<li class="options__empty">{t('restore.facetEmpty')}</li>
				{/each}
			</ul>
		</Popover>

		<Popover label={t('restore.facetTag')} bind:open={tagOpen}>
			{#snippet trigger()}
				<span class="facet" class:facet--active={tags.length > 0}>
					{t('restore.facetTag')}
					{#if tags.length > 0}<span class="facet__count">{tags.length}</span>{/if}
					<Icon name="chevron-down" size="xs" />
				</span>
			{/snippet}
			<ul class="options">
				{#each tagOptions as option (option.value)}
					<li class="option">
						<Checkbox
							label={tagLabel(option.value, profiles)}
							checked={tags.includes(option.value)}
							onchange={() => onchange({ tag: joinList(toggle(tags, option.value)) })}
						/>
						<span class="option__count">{option.count}</span>
					</li>
				{:else}
					<li class="options__empty">{t('restore.facetEmpty')}</li>
				{/each}
			</ul>
		</Popover>

		<Popover label={t('restore.facetPeriod')} bind:open={periodOpen}>
			{#snippet trigger()}
				<span class="facet" class:facet--active={period !== ''}>
					{t('restore.facetPeriod')}
					<Icon name="chevron-down" size="xs" />
				</span>
			{/snippet}
			<ul class="options">
				{#each PERIOD_PRESETS as preset (preset)}
					<li>
						<button
							type="button"
							class="choice"
							aria-pressed={period === preset}
							onclick={() => choosePeriod(preset)}
						>
							{t(`restore.period.${preset}`)}
							{#if period === preset}<Icon name="check" size="md" />{/if}
						</button>
					</li>
				{/each}
				<li>
					<button
						type="button"
						class="choice"
						aria-pressed={period === ''}
						onclick={() => choosePeriod('')}
					>
						{t('restore.period.all')}
						{#if period === ''}<Icon name="check" size="md" />{/if}
					</button>
				</li>
				<li>
					<button
						type="button"
						class="choice"
						aria-pressed={period === CUSTOM_PERIOD}
						onclick={() => choosePeriod(CUSTOM_PERIOD)}
					>
						{t('restore.period.custom')}
						{#if period === CUSTOM_PERIOD}<Icon name="check" size="md" />{/if}
					</button>
				</li>
			</ul>
			{#if period === CUSTOM_PERIOD}
				<div class="range">
					<TextField
						type="date"
						label={t('restore.periodFrom')}
						value={filters.from}
						max={filters.to || undefined}
						oninput={(event) =>
							onchange({ period: CUSTOM_PERIOD, from: event.currentTarget.value })}
					/>
					<TextField
						type="date"
						label={t('restore.periodTo')}
						value={filters.to}
						min={filters.from || undefined}
						oninput={(event) => onchange({ period: CUSTOM_PERIOD, to: event.currentTarget.value })}
					/>
				</div>
			{/if}
		</Popover>

		<Tooltip content={t('restore.filterHint')} ariaLabel={t('restore.filterHint')}>
			<span class="hint"><Icon name="info" size="md" /></span>
		</Tooltip>

		{#if hasFilters(filters)}
			<Button variant="ghost" size="sm" onclick={onreset}>{t('restore.clearFilters')}</Button>
		{/if}
	</div>

	{#if chips.length > 0}
		<ul class="chips">
			{#each chips as chip (chip.key)}
				<li class="chip">
					<span class="chip__text">{chip.text}</span>
					<button
						type="button"
						class="chip__remove"
						aria-label={t('restore.chipRemove', { name: chip.text })}
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
	.bar {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-2);
		padding: var(--rh-space-3) var(--rh-space-4);
		border-bottom: var(--rh-border-width) solid var(--rh-color-border-muted);
	}

	.bar__row {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.bar__search {
		position: relative;
		flex: 1 1 14rem;
		max-width: 26rem;
	}

	.bar__search-icon {
		position: absolute;
		top: 50%;
		left: var(--rh-space-3);
		z-index: 1;
		display: inline-flex;
		transform: translateY(-50%);
		color: var(--rh-color-text-muted);
		pointer-events: none;
	}

	.bar__search :global(.bar__input) {
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
