<script lang="ts">
	import Dialog from '#lib/components/Dialog.svelte';

	let { open = $bindable(false) } = $props();
</script>

<Dialog bind:open title="Test dialog" description="A description">
	<button type="button">First action</button>
	<button type="button">Second action</button>
</Dialog>
