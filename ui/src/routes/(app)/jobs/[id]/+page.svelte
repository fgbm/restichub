<script lang="ts">
	import { page } from '$app/state';
	import JobPage from '#lib/components/JobPage.svelte';

	// A new job in the same route starts with a fresh page: the log of the previous run, its steps
	// and its subscriptions belong to that run only.
	const id = $derived(page.params.id ?? '');
</script>

{#key id}
	<JobPage jobId={id} />
{/key}
