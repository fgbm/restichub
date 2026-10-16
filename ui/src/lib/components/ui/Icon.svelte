<script lang="ts">
	import { icons, type IconName } from './icons';

	type Size = 'xs' | 'md' | 'lg' | 'xl';

	let {
		name,
		size = 'md',
		label,
		spin = false,
		class: className = ''
	}: {
		name: IconName;
		size?: Size;
		/** Names the icon for assistive technology; without it the icon is decorative. */
		label?: string;
		spin?: boolean;
		class?: string;
	} = $props();

	const shapes = $derived(icons[name]);
</script>

<svg
	class="icon icon--{size} {className}"
	class:icon--spin={spin}
	viewBox="0 0 24 24"
	focusable="false"
	role={label ? 'img' : undefined}
	aria-label={label}
	aria-hidden={label ? undefined : 'true'}
	data-icon={name}
>
	{#each shapes as [tag, attributes], index (index)}
		{#if tag === 'path'}
			<path {...attributes} />
		{:else if tag === 'circle'}
			<circle {...attributes} />
		{:else if tag === 'rect'}
			<rect {...attributes} />
		{:else if tag === 'ellipse'}
			<ellipse {...attributes} />
		{:else if tag === 'line'}
			<line {...attributes} />
		{:else if tag === 'polyline'}
			<polyline {...attributes} />
		{:else}
			<polygon {...attributes} />
		{/if}
	{/each}
</svg>

<style>
	.icon {
		flex: none;
		width: 16px;
		height: 16px;
		fill: none;
		stroke: currentColor;
		stroke-width: 2;
		stroke-linecap: round;
		stroke-linejoin: round;
		vertical-align: -0.15em;
	}

	.icon--xs {
		width: 14px;
		height: 14px;
	}

	.icon--lg {
		width: 18px;
		height: 18px;
	}

	.icon--xl {
		width: 24px;
		height: 24px;
	}

	.icon--spin {
		animation: icon-spin var(--rh-spin-duration) linear infinite;
	}

	@keyframes icon-spin {
		to {
			transform: rotate(360deg);
		}
	}
</style>
