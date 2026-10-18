<script lang="ts">
	import Button from './Button.svelte';
	import type { DangerAction } from './types';

	let { actions }: { actions: DangerAction[] } = $props();
</script>

<ul class="danger">
	{#each actions as action (action.key)}
		<li>
			<p class="lead">{action.text}</p>
			<Button
				variant={action.tone ?? 'secondary'}
				icon={action.icon}
				disabled={action.disabled}
				loading={action.loading}
				title={action.hint}
				onclick={action.onclick}
			>
				{action.label}
			</Button>
		</li>
	{/each}
</ul>

<style>
	.danger {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-4);
		margin: 0;
		padding: 0;
		list-style: none;
	}

	.danger li {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: var(--rh-space-3);
	}

	.lead {
		flex: 1 1 260px;
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}
</style>
