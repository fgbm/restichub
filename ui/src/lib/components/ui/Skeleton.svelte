<script lang="ts">
	type Variant = 'text' | 'row' | 'avatar';

	let { variant = 'text' }: { variant?: Variant } = $props();
</script>

<span class="skeleton skeleton--{variant}" aria-hidden="true"></span>

<style>
	.skeleton {
		display: block;
		background: linear-gradient(
			90deg,
			var(--rh-color-skeleton),
			var(--rh-color-skeleton-shine),
			var(--rh-color-skeleton)
		);
		background-size: 200% 100%;
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
			background-position: -200% 0;
		}
	}
</style>
