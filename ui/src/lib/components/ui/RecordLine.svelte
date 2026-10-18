<script lang="ts">
	import Badge from './Badge.svelte';
	import type { RecordPart } from './types';

	let { parts, id }: { parts: readonly RecordPart[]; id?: string } = $props();
</script>

<span class="record-line" {id}>
	{#each parts as part, index (index)}
		{#if typeof part === 'string'}
			{#if index > 0}<span class="record-line__sep" aria-hidden="true">·</span>{/if}
			<span class="record-line__text">{part}</span>
		{:else}
			<Badge icon={part.icon}>{part.tag}</Badge>
		{/if}
	{/each}
</span>

<style>
	.record-line {
		display: flex;
		align-items: center;
		gap: var(--rh-space-1);
		min-width: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
		white-space: nowrap;
	}

	.record-line__text {
		overflow: hidden;
		text-overflow: ellipsis;
	}
</style>
