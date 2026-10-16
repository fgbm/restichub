<script lang="ts">
	import { page } from '$app/state';
	import HostPage from '#lib/components/HostPage.svelte';

	const id = $derived(page.params.id ?? '');
</script>

{#key id}
	<HostPage hostId={id} />
{/key}
