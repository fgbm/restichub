<script lang="ts">
	import FilterBar from '#lib/components/ui/FilterBar.svelte';
	import { queryFilters } from '#lib/stores/query-filters.svelte.ts';

	let { hosts = ['h1', 'h2'], hostsLoaded = true }: { hosts?: string[]; hostsLoaded?: boolean } =
		$props();

	const filters = queryFilters(['q', 'host', 'state', 'sort', 'order', 'page', 'size'] as const);
</script>

<output data-testid="state">{JSON.stringify(filters.value)}</output>
<output data-testid="restored">{filters.restored}</output>
<FilterBar
	label="Probe"
	value={filters.value}
	onchange={filters.update}
	onreset={filters.reset}
	states={{
		default: 'all',
		options: [
			{ value: 'all', label: 'All' },
			{ value: 'failed', label: 'Failed' }
		]
	}}
	sort={{
		default: 'name',
		options: [
			{ value: 'name', label: 'Name' },
			{ value: 'host', label: 'Host' }
		]
	}}
	facets={[
		{
			key: 'host',
			label: 'Host',
			exact: hostsLoaded,
			options: hosts.map((value) => ({ value }))
		}
	]}
/>
<button onclick={() => filters.update({ q: 'text', page: '2' })}>search</button>
<button
	onclick={() => filters.update({ state: 'failed', size: '50', sort: 'host', order: 'desc' })}
>
	view
</button>
<button onclick={() => filters.update({ host: 'h2' })}>host</button>
