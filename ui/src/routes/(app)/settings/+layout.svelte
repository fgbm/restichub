<script lang="ts">
	import type { Snippet } from 'svelte';
	import { SettingsLayout } from '#lib/components/index.ts';

	let { children }: { children: Snippet } = $props();
</script>

<!-- The section keeps its subnav on every screen, so a record and its list start at the same
     place (design/style/ui-principles.md, 1.3). -->
<SettingsLayout>{@render children()}</SettingsLayout>
