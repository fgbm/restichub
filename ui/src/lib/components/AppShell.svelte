<script lang="ts">
	import type { Snippet } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { service } from '#lib/api/index.ts';
	import type { Role } from '#lib/api/index.ts';
	import {
		getLocale,
		locales,
		setLocale,
		t,
		type Locale,
		type MessageKey
	} from '#lib/i18n/index.ts';
	import { navSectionsForRole, type NavHref } from '#lib/navigation.ts';
	import { isSingleTenant } from '#lib/stores/instance.svelte.ts';
	import { clearSession, getSession } from '#lib/stores/session.svelte.ts';
	import { getTheme, setTheme, themes, type Theme } from '#lib/stores/theme.svelte.ts';
	import Toast from './Toast.svelte';

	let { children }: { children: Snippet } = $props();

	const roleKeys = $derived<Record<Role, MessageKey>>({
		platform_admin: 'users.role.platformAdmin',
		admin: 'users.role.admin',
		operator: 'users.role.operator',
		viewer: 'users.role.viewer'
	});

	const session = $derived(getSession());
	// Single mode hides the platform section (ADR-0024).
	const sections = $derived(
		navSectionsForRole(session?.role ?? null).filter(
			(section) => !(isSingleTenant() && section.platform)
		)
	);
	const pathname = $derived(page.url.pathname);
	const locale = $derived(getLocale());
	const theme = $derived(getTheme());

	function isActive(href: NavHref): boolean {
		const target = resolve(href);
		return target === '/' ? pathname === '/' : pathname.startsWith(target);
	}

	async function signOut(): Promise<void> {
		try {
			await service.signOut();
		} catch {
			// The session ends on the client even if the call fails.
		}
		clearSession();
		await goto(resolve('/(auth)/sign-in'));
	}
</script>

<a class="skip-link" href="#main">{t('app.skipToContent')}</a>

<div class="shell">
	<header class="topbar">
		<div class="topbar__brand">
			<span class="topbar__product">{t('app.name')}</span>
			{#if !isSingleTenant()}
				<span class="topbar__tenant">{session?.tenant?.name ?? t('topbar.noTenant')}</span>
			{/if}
		</div>

		<details class="menu">
			<summary class="menu__summary" aria-label={t('topbar.account')}>
				<span class="menu__user">{session?.user?.display_name ?? t('topbar.account')}</span>
				{#if session}
					<span class="menu__role">{t(roleKeys[session.role])}</span>
				{/if}
			</summary>
			<div class="menu__panel">
				<p class="menu__group-title">{t('topbar.theme')}</p>
				<div class="menu__group" role="group" aria-label={t('topbar.theme')}>
					{#each themes as option (option)}
						<button
							type="button"
							class="menu__option"
							aria-pressed={theme === option}
							onclick={() => setTheme(option as Theme)}
						>
							{t(`topbar.theme.${option}` as MessageKey)}
						</button>
					{/each}
				</div>

				<p class="menu__group-title">{t('topbar.language')}</p>
				<div class="menu__group" role="group" aria-label={t('topbar.language')}>
					{#each locales as option (option)}
						<button
							type="button"
							class="menu__option"
							aria-pressed={locale === option}
							onclick={() => setLocale(option as Locale)}
						>
							{option === 'ru' ? 'Русский' : 'English'}
						</button>
					{/each}
				</div>

				<hr class="menu__divider" />
				<button type="button" class="menu__option menu__option--action" onclick={signOut}>
					{t('topbar.signOut')}
				</button>
			</div>
		</details>
	</header>

	<div class="shell__body">
		<nav class="nav" aria-label={t('nav.primary')}>
			{#each sections as section (section.labelKey)}
				<div class="nav__section">
					<p class="nav__section-title">{t(section.labelKey)}</p>
					<ul class="nav__list">
						{#each section.items as item (item.href)}
							<li>
								<a
									class="nav__link"
									class:nav__link--active={isActive(item.href)}
									href={resolve(item.href)}
									aria-current={isActive(item.href) ? 'page' : undefined}
								>
									{t(item.labelKey)}
								</a>
							</li>
						{/each}
					</ul>
				</div>
			{/each}
		</nav>

		<main id="main" class="content">
			<div class="content__inner">{@render children()}</div>
		</main>
	</div>
</div>

<Toast />

<style>
	.skip-link {
		position: absolute;
		left: -9999px;
		top: 0;
		padding: var(--rh-space-2) var(--rh-space-3);
		background-color: var(--rh-color-surface);
		color: var(--rh-color-text);
		z-index: var(--rh-z-toast);
	}

	.skip-link:focus {
		left: var(--rh-space-2);
		top: var(--rh-space-2);
	}

	.shell {
		display: flex;
		flex-direction: column;
		min-height: 100vh;
	}

	.topbar {
		position: sticky;
		top: 0;
		z-index: var(--rh-z-topbar);
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--rh-space-4);
		height: var(--rh-layout-topbar-height);
		padding: 0 var(--rh-space-4);
		background-color: var(--rh-color-surface);
		border-bottom: var(--rh-border-width) solid var(--rh-color-border);
	}

	.topbar__brand {
		display: flex;
		align-items: baseline;
		gap: var(--rh-space-3);
	}

	.topbar__product {
		font-size: var(--rh-font-size-lg);
		font-weight: var(--rh-font-weight-semibold);
	}

	.topbar__tenant {
		font-size: var(--rh-font-size-sm);
		color: var(--rh-color-text-muted);
	}

	.menu {
		position: relative;
	}

	.menu__summary {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
		padding: var(--rh-space-1) var(--rh-space-2);
		border: var(--rh-border-width) solid transparent;
		border-radius: var(--rh-radius-sm);
		cursor: pointer;
		list-style: none;
	}

	.menu__summary::-webkit-details-marker {
		display: none;
	}

	.menu__summary:hover {
		background-color: var(--rh-color-hover);
	}

	.menu__user {
		font-weight: var(--rh-font-weight-medium);
	}

	.menu__role {
		font-size: var(--rh-font-size-xs);
		color: var(--rh-color-text-muted);
	}

	.menu__panel {
		position: absolute;
		right: 0;
		top: calc(100% + var(--rh-space-1));
		min-width: 14rem;
		padding: var(--rh-space-2);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-surface);
		box-shadow: var(--rh-shadow-overlay);
	}

	.menu__group-title {
		margin: var(--rh-space-2) var(--rh-space-2) var(--rh-space-1);
		font-size: var(--rh-font-size-xs);
		text-transform: uppercase;
		letter-spacing: 0.04em;
		color: var(--rh-color-text-subtle);
	}

	.menu__group {
		display: flex;
		flex-wrap: wrap;
		gap: var(--rh-space-1);
		padding: 0 var(--rh-space-2);
	}

	.menu__option {
		padding: var(--rh-space-1) var(--rh-space-2);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-sm);
		background-color: var(--rh-color-surface);
		color: var(--rh-color-text);
		font-size: var(--rh-font-size-sm);
		cursor: pointer;
	}

	.menu__option[aria-pressed='true'] {
		border-color: var(--rh-color-accent);
		background-color: var(--rh-color-accent-subtle);
		color: var(--rh-color-accent-subtle-text);
	}

	.menu__option--action {
		width: 100%;
		border: 0;
		background: transparent;
		text-align: left;
	}

	.menu__option--action:hover {
		background-color: var(--rh-color-hover);
	}

	.menu__divider {
		margin: var(--rh-space-2) 0;
		border: 0;
		border-top: var(--rh-border-width) solid var(--rh-color-border);
	}

	.shell__body {
		display: flex;
		flex: 1;
		align-items: stretch;
	}

	.nav {
		width: var(--rh-layout-nav-width);
		flex-shrink: 0;
		padding: var(--rh-space-4) var(--rh-space-3);
		border-right: var(--rh-border-width) solid var(--rh-color-border);
		background-color: var(--rh-color-surface);
	}

	.nav__section + .nav__section {
		margin-top: var(--rh-space-5);
	}

	.nav__section-title {
		margin: 0 0 var(--rh-space-1);
		padding: 0 var(--rh-space-2);
		font-size: var(--rh-font-size-xs);
		text-transform: uppercase;
		letter-spacing: 0.04em;
		color: var(--rh-color-text-subtle);
	}

	.nav__list {
		margin: 0;
		padding: 0;
		list-style: none;
	}

	.nav__link {
		display: block;
		padding: var(--rh-space-2);
		border-radius: var(--rh-radius-sm);
		color: var(--rh-color-text);
		font-size: var(--rh-font-size-base);
	}

	.nav__link:hover {
		background-color: var(--rh-color-hover);
		text-decoration: none;
	}

	.nav__link--active {
		background-color: var(--rh-color-accent-subtle);
		color: var(--rh-color-accent-subtle-text);
		font-weight: var(--rh-font-weight-medium);
	}

	.content {
		flex: 1;
		min-width: 0;
		padding: var(--rh-space-6) var(--rh-space-6);
	}

	.content__inner {
		max-width: var(--rh-layout-content-max-width);
		margin: 0 auto;
	}

	@media (max-width: 720px) {
		.shell__body {
			flex-direction: column;
		}

		.nav {
			width: 100%;
			border-right: 0;
			border-bottom: var(--rh-border-width) solid var(--rh-color-border);
		}
	}
</style>
