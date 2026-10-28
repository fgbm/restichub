<script lang="ts">
	import type { Snippet } from 'svelte';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { t } from '#lib/i18n/index.ts';
	import { settingsGroupsForRole } from '#lib/navigation.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { Icon } from './ui/index.ts';

	let { children }: { children: Snippet } = $props();

	const groups = $derived(settingsGroupsForRole(getSession()?.role ?? null));
	const pathname = $derived(page.url.pathname);

	const activeHref = $derived(
		groups
			.flatMap((group) => group.items)
			.map((item) => resolve(item.href))
			.find((href) => pathname.startsWith(href))
	);

	let subnav = $state<HTMLElement>();
	let rowWidth = $state(0);

	// The window scrollbar changes the width of the row, and the current entry moves with it.
	$effect(() => {
		if (!subnav || typeof ResizeObserver === 'undefined') return;
		const observer = new ResizeObserver(() => (rowWidth = subnav?.clientWidth ?? 0));
		observer.observe(subnav);
		return () => observer.disconnect();
	});

	// A narrow window shows the subnav as one row, and the current entry may be outside it.
	$effect(() => {
		const current =
			activeHref && rowWidth ? subnav?.querySelector<HTMLElement>(`a[href="${activeHref}"]`) : null;
		if (!subnav || !current || subnav.scrollWidth <= subnav.clientWidth) return;
		subnav.scrollLeft +=
			current.getBoundingClientRect().left -
			subnav.getBoundingClientRect().left -
			(subnav.clientWidth - current.offsetWidth) / 2;
	});
</script>

<div class="settings" class:settings--no-subnav={groups.length === 0}>
	{#if groups.length > 0}
		<nav class="subnav" aria-label={t('settings.sections')} bind:this={subnav}>
			{#each groups as group (group.labelKey)}
				<div class="subnav__group">
					<p class="subnav__label">{t(group.labelKey)}</p>
					{#each group.items as item (item.href)}
						<a
							class="subnav__link"
							href={resolve(item.href)}
							aria-current={activeHref === resolve(item.href) ? 'page' : undefined}
						>
							<Icon name={item.icon} />
							{t(item.labelKey)}
						</a>
					{/each}
				</div>
			{/each}
		</nav>
	{/if}
	<div class="settings__content">
		{@render children()}
	</div>
</div>

<style>
	.settings {
		display: grid;
		grid-template-columns: 200px minmax(0, 1fr);
		gap: var(--rh-space-8);
		align-items: start;
	}

	/* A role without settings screens has no subnav, so the content gets the full width. */
	.settings--no-subnav {
		grid-template-columns: minmax(0, 1fr);
	}

	.subnav {
		position: sticky;
		top: calc(var(--rh-layout-topbar-height) + var(--rh-space-4));
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-1);
	}

	.subnav__group + .subnav__group {
		margin-top: var(--rh-space-3);
	}

	.subnav__label {
		margin: 0;
		padding: var(--rh-space-2) var(--rh-space-2) var(--rh-space-1);
		color: var(--rh-color-text-subtle);
		font-size: var(--rh-font-size-xs);
		font-weight: var(--rh-font-weight-semibold);
	}

	.subnav__link {
		display: flex;
		align-items: center;
		gap: var(--rh-space-3);
		height: var(--rh-control-height);
		padding: 0 var(--rh-space-2);
		border-radius: var(--rh-radius-md);
		color: var(--rh-color-text-muted);
	}

	.subnav__link:hover {
		background-color: var(--rh-color-hover);
		color: var(--rh-color-text);
		text-decoration: none;
	}

	.subnav__link[aria-current='page'] {
		background-color: var(--rh-color-hover);
		color: var(--rh-color-text);
		font-weight: var(--rh-font-weight-semibold);
	}

	.settings__content {
		display: flex;
		flex-direction: column;
		min-width: 0;
		/* The column is the size that a screen inside it works with. A record page stacks its side
		   panel when this column is narrow, also in a wide window. */
		container: settings-column / inline-size;
	}

	@media (max-width: 900px) {
		.settings {
			grid-template-columns: minmax(0, 1fr);
		}

		/* A narrow window shows the subnav as one row that scrolls sideways, with the look of the
		   tabs of the top bar. Thus the heading stays at the top of the screen. */
		.subnav {
			position: static;
			flex-direction: row;
			gap: var(--rh-space-1);
			overflow-x: auto;
			overflow-y: hidden;
			scrollbar-width: none;
			box-shadow: inset 0 calc(-1 * var(--rh-border-width)) 0 var(--rh-color-border);
		}

		.subnav::-webkit-scrollbar {
			display: none;
		}

		.subnav__group {
			display: flex;
			flex: none;
			gap: var(--rh-space-1);
		}

		.subnav__group + .subnav__group {
			margin-top: 0;
		}

		.subnav__label {
			display: none;
		}

		.subnav__link {
			flex: none;
			height: 44px;
			padding: 0 var(--rh-space-3);
			border-bottom: 2px solid transparent;
			border-radius: 0;
			white-space: nowrap;
		}

		.subnav__link:hover {
			background-color: transparent;
			border-bottom-color: var(--rh-color-border-strong);
		}

		.subnav__link[aria-current] {
			background-color: transparent;
			border-bottom-color: var(--rh-color-accent);
		}
	}
</style>
