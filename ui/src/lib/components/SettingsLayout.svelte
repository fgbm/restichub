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
</script>

<div class="settings">
	<nav class="subnav" aria-label={t('settings.sections')}>
		{#each groups as group (group.labelKey)}
			<div class="subnav__group">
				<p class="subnav__label">{t(group.labelKey)}</p>
				{#each group.items as item (item.href)}
					<a
						class="subnav__link"
						href={resolve(item.href)}
						aria-current={pathname.startsWith(resolve(item.href)) ? 'page' : undefined}
					>
						<Icon name={item.icon} />
						{t(item.labelKey)}
					</a>
				{/each}
			</div>
		{/each}
	</nav>
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
		gap: var(--rh-space-4);
		min-width: 0;
		/* The column is the size that a screen inside it works with. A record page stacks its side
		   panel when this column is narrow, also in a wide window. */
		container: settings-column / inline-size;
	}

	@media (max-width: 900px) {
		.settings {
			grid-template-columns: minmax(0, 1fr);
		}

		.subnav {
			position: static;
		}
	}
</style>
