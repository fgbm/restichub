<script lang="ts">
	type Variant = 'text' | 'row' | 'avatar';

	let { variant = 'text' }: { variant?: Variant } = $props();
</script>

<span class="skeleton skeleton--{variant}" aria-hidden="true"></span>

<style>
	/* The shine moves with a transform, so it runs on the compositor and does not stop while the
	   main thread renders the data that the skeleton waits for. */
	.skeleton {
		position: relative;
		display: block;
		overflow: hidden;
		background: var(--rh-color-skeleton);
	}

	.skeleton::after {
		content: '';
		position: absolute;
		inset: 0;
		background: linear-gradient(90deg, transparent, var(--rh-color-skeleton-shine), transparent);
		transform: translateX(-100%);
		animation: skeleton-shimmer 1.2s ease-in-out infinite;
	}

	.skeleton--text {
		width: 100%;
		height: 0.85em;
		border-radius: var(--rh-radius-sm);
	}

	.skeleton--row {
		width: 100%;
		height: 2.5rem;
		border-radius: var(--rh-radius-md);
	}

	.skeleton--avatar {
		width: 24px;
		height: 24px;
		border-radius: var(--rh-radius-pill);
	}

	@keyframes skeleton-shimmer {
		to {
			transform: translateX(100%);
		}
	}
</style>
