<script lang="ts">
	import { asset } from '$app/paths';
	import type { IconName } from './icons';

	type Size = 'xs' | 'md' | 'lg' | 'xl';

	let {
		name,
		size = 'md',
		label
	}: {
		name: IconName;
		size?: Size;
		label?: string;
	} = $props();

	const href = $derived(`${asset('icons.svg')}#${name}`);
</script>

<svg
	class="icon icon--{size}"
	role={label ? 'img' : undefined}
	aria-label={label}
	aria-hidden={label ? undefined : 'true'}
>
	<use {href} />
</svg>

<style>
	.icon {
		display: inline-block;
		width: 16px;
		height: 16px;
		flex: none;
		fill: none;
		stroke: currentColor;
		stroke-width: 1.5;
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
</style>
