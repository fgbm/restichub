<script lang="ts">
	let { size = 'md' }: { size?: 'md' | 'lg' } = $props();
</script>

<span class="brandmark brandmark--{size}" aria-hidden="true">
	<svg viewBox="0 0 20 20">
		<path class="brandmark__solid" d="M10 2.5 2.5 6.4 10 10.3l7.5-3.9z" />
		<path d="M2.5 10.6 10 14.5l7.5-3.9" />
	</svg>
</span>

<style>
	.brandmark {
		display: inline-flex;
		flex: none;
		align-items: center;
		justify-content: center;
		width: 26px;
		height: 26px;
		border-radius: 7px;
		background-color: var(--rh-color-accent);
		color: var(--rh-color-text-on-accent);
	}

	.brandmark--lg {
		width: 44px;
		height: 44px;
		border-radius: 11px;
	}

	svg {
		width: 16px;
		height: 16px;
		fill: none;
		stroke: currentColor;
		stroke-width: 1.7;
		stroke-linecap: round;
		stroke-linejoin: round;
	}

	.brandmark--lg svg {
		width: 26px;
		height: 26px;
	}

	.brandmark__solid {
		fill: currentColor;
		stroke: none;
	}
</style>
