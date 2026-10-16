<script lang="ts">
	import { page } from '$app/state';
	import PlatformUserPage from '#lib/components/PlatformUserPage.svelte';

	const id = $derived(page.params.id ?? '');
</script>

{#key id}
	<PlatformUserPage userId={id} />
{/key}
