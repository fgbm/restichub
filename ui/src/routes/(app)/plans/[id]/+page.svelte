<script lang="ts">
	import { page } from '$app/state';
	import PlanEditor from '#lib/components/PlanEditor.svelte';

	const id = $derived(page.params.id ?? null);
</script>

{#key id}
	<PlanEditor planId={id} />
{/key}
