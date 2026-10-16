<script lang="ts">
	import { FilterBar } from '#lib/components/ui/index.ts';
	import type { FilterFacet } from '#lib/components/ui/index.ts';
	import { t } from '#lib/i18n/index.ts';
	import type { SnapshotItem } from '#lib/api/index.ts';
	import type { RestoreFilters } from '#lib/restore.ts';
	import { facetCounts, tagLabel } from '#lib/snapshot-filter.ts';

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

	const facets = $derived<FilterFacet[]>([
		{
			key: 'host',
			label: t('restore.facetHost'),
			options: facetCounts(snapshots, (item) => [item.hostname])
		},
		{
			key: 'tag',
			label: t('restore.facetTag'),
			options: facetCounts(snapshots, (item) => item.tags).map((option) => ({
				...option,
				label: tagLabel(option.value, profiles)
			}))
		}
	]);
</script>

<FilterBar
	label={t('restore.filterBar')}
	value={{ ...filters }}
	search={{
		key: 'path',
		id: 'restore-path',
		label: t('restore.pathSearch'),
		placeholder: t('restore.pathPlaceholder'),
		mono: true
	}}
	{facets}
	period
	hint={t('restore.filterHint')}
	{onchange}
	{onreset}
/>
