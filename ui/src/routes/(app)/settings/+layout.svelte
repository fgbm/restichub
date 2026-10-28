<script lang="ts">
	import type { Snippet } from 'svelte';
	import SettingsLayout from '#lib/components/SettingsLayout.svelte';

	let { children }: { children: Snippet } = $props();
</script>

<!-- The subnav of the section stays on each page of it, so a record and its list start at the
     same place (design/style/ui-principles.md, 1.8). -->
<SettingsLayout>{@render children()}</SettingsLayout>
