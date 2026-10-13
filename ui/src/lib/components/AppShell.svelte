<script lang="ts">
	import type { Snippet } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { service } from '#lib/api/index.ts';
	import { getLocale, setLocale, t } from '#lib/i18n/index.ts';
	import { navSectionsForRole, type NavHref } from '#lib/navigation.ts';
	import { isSingleTenant } from '#lib/stores/instance.svelte.ts';
	import { clearSession, getSession } from '#lib/stores/session.svelte.ts';
	import { getTheme, setTheme } from '#lib/stores/theme.svelte.ts';
	import { DropdownMenu, ToastRegion } from './ui/index.ts';
	import type { MenuItem } from './ui/index.ts';
	import TenantSwitcher from './TenantSwitcher.svelte';

	let { children }: { children: Snippet } = $props();

	const session = $derived(getSession());
	// Single mode hides the platform section (ADR-0024). The platform section appears with the
	// Platform Admin right; the administration section needs a current organization (ADR-0029).
	const sections = $derived(
		navSectionsForRole(session?.role ?? null, session?.platform_admin ?? false).filter(
			(section) =>
				!(isSingleTenant() && section.platform) && (section.platform || !!session?.tenant)
		)
	);
	const pathname = $derived(page.url.pathname);
	const locale = $derived(getLocale());
	const theme = $derived(getTheme());

	const displayName = $derived(session?.user?.display_name ?? t('topbar.account'));
	const initials = $derived(
		displayName
			.split(/\s+/)
			.filter(Boolean)
			.slice(0, 2)
			.map((part) => part[0]?.toUpperCase() ?? '')
			.join('')
	);

	const accountItems = $derived<MenuItem[]>([
		// The Platform Admin right appears on the account menu, not as an organization (ADR-0029).
		...(session?.platform_admin ? [{ label: t('users.role.platformAdmin') }] : []),
		{
			label: t('topbar.theme.light'),
			group: 'theme',
			checked: theme === 'light',
			onSelect: () => setTheme('light')
		},
		{
			label: t('topbar.theme.dark'),
			group: 'theme',
			checked: theme === 'dark',
			onSelect: () => setTheme('dark')
		},
		{
			label: t('topbar.theme.system'),
			group: 'theme',
			checked: theme === 'system',
			onSelect: () => setTheme('system')
		},
		{
			label: 'English',
			group: 'language',
			checked: locale === 'en',
			separatorBefore: true,
			onSelect: () => setLocale('en')
		},
		{
			label: 'Русский',
			group: 'language',
			checked: locale === 'ru',
			onSelect: () => setLocale('ru')
		},
		{
			label: t('topbar.signOut'),
			tone: 'danger',
			separatorBefore: true,
			onSelect: signOut
		}
	]);

	function isActive(href: NavHref): boolean {
		const target = resolve(href);
		return target === '/' ? pathname === '/' : pathname.startsWith(target);
	}

	function toggleTheme(): void {
		setTheme(theme === 'dark' ? 'light' : 'dark');
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
		<a class="brand" href={resolve('/(app)')}>
			<span class="brandmark" aria-hidden="true">
				<svg viewBox="0 0 20 20">
					<path class="brandmark__solid" d="M10 2.5 2.5 6.4 10 10.3l7.5-3.9z" />
					<path d="M2.5 10.6 10 14.5l7.5-3.9" />
				</svg>
			</span>
			<span>{t('app.name')}</span>
		</a>

		<span class="topbar__divider"></span>

		{#if isSingleTenant()}
			<span class="topbar__tenant">{session?.tenant?.name ?? t('topbar.noTenant')}</span>
		{:else}
			<TenantSwitcher />
		{/if}

		<div class="spacer"></div>

		<button type="button" class="icon-btn" aria-label={t('topbar.theme')} onclick={toggleTheme}>
			<span aria-hidden="true">{theme === 'dark' ? '☀' : '☾'}</span>
		</button>

		<DropdownMenu items={accountItems} align="end">
			<span class="account">
				<span class="avatar" aria-hidden="true">{initials}</span>
				<span class="account__name">{displayName}</span>
				<span aria-hidden="true">▾</span>
			</span>
		</DropdownMenu>
	</header>

	<nav class="tabs" aria-label={t('nav.primary')}>
		{#each sections as section, index (section.labelKey)}
			{#if section.platform && index > 0}
				<span class="tabs__divider" aria-hidden="true"></span>
			{/if}
			{#each section.items as item (item.href)}
				<a
					class="tab"
					class:tab--platform={section.platform}
					href={resolve(item.href)}
					aria-current={isActive(item.href) ? 'page' : undefined}
				>
					{t(item.labelKey)}
				</a>
			{/each}
		{/each}
	</nav>

	<main id="main" class="page">
		<div class="page__inner">
			<!-- A tenant switch remounts the screen, so its data reloads for the new tenant. -->
			{#key session?.tenant?.id}
				{@render children()}
			{/key}
		</div>
	</main>
</div>

<ToastRegion />

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
		gap: var(--rh-space-3);
		height: var(--rh-layout-topbar-height);
		padding: 0 var(--rh-space-4);
		background-color: var(--rh-color-canvas);
		border-bottom: var(--rh-border-width) solid var(--rh-color-border);
	}

	.brand {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-2);
		color: var(--rh-color-text);
		font-weight: var(--rh-font-weight-semibold);
	}

	.brand:hover {
		text-decoration: none;
	}

	.brandmark {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 26px;
		height: 26px;
		border-radius: 7px;
		background-color: var(--rh-color-accent);
		color: var(--rh-color-text-on-accent);
	}

	.brandmark svg {
		width: 16px;
		height: 16px;
		fill: none;
		stroke: currentColor;
		stroke-width: 1.7;
		stroke-linejoin: round;
		stroke-linecap: round;
	}

	.brandmark__solid {
		fill: currentColor;
		stroke: none;
	}

	.topbar__divider {
		width: 1px;
		height: 22px;
		background-color: var(--rh-color-border);
	}

	.topbar__tenant {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-base);
	}

	.spacer {
		flex: 1 1 auto;
	}

	.icon-btn {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: var(--rh-control-height);
		height: var(--rh-control-height);
		border: var(--rh-border-width) solid transparent;
		border-radius: var(--rh-radius-md);
		background: transparent;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-md);
		cursor: pointer;
	}

	.icon-btn:hover {
		background-color: var(--rh-color-hover);
		color: var(--rh-color-text);
	}

	.account {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.avatar {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 24px;
		height: 24px;
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-pill);
		background-color: var(--rh-color-neutral-bg);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-2xs);
		font-weight: var(--rh-font-weight-semibold);
	}

	.account__name {
		font-weight: var(--rh-font-weight-medium);
	}

	.tabs {
		display: flex;
		align-items: stretch;
		gap: var(--rh-space-1);
		padding: 0 var(--rh-space-4);
		background-color: var(--rh-color-canvas);
		/* The bar border is an inset shadow, so the active tab border covers it instead of a
		   negative margin that would add 1px of vertical overflow and a scrollbar (classic ones). */
		box-shadow: inset 0 calc(-1 * var(--rh-border-width)) 0 var(--rh-color-border);
		overflow-x: auto;
		overflow-y: hidden;
		scrollbar-width: thin;
	}

	.tab {
		display: inline-flex;
		align-items: center;
		height: 44px;
		padding: 0 var(--rh-space-3);
		border-bottom: 2px solid transparent;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-base);
		white-space: nowrap;
	}

	.tab:hover {
		color: var(--rh-color-text);
		border-bottom-color: var(--rh-color-border-strong);
		text-decoration: none;
	}

	.tab[aria-current='page'] {
		color: var(--rh-color-text);
		font-weight: var(--rh-font-weight-semibold);
		border-bottom-color: var(--rh-color-accent);
	}

	.tabs__divider {
		align-self: center;
		width: 1px;
		height: 22px;
		margin: 0 var(--rh-space-2);
		background-color: var(--rh-color-border);
	}

	.tab--platform {
		color: var(--rh-color-accent);
	}

	.tab--platform:hover,
	.tab--platform[aria-current='page'] {
		color: var(--rh-color-accent);
	}

	.page {
		flex: 1;
		min-width: 0;
		padding: var(--rh-space-6) var(--rh-space-4) var(--rh-space-12);
	}

	.page__inner {
		max-width: var(--rh-layout-content-max-width);
		margin: 0 auto;
	}
</style>
