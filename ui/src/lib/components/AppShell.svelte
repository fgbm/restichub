<script lang="ts">
	import type { Snippet } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { service } from '#lib/api/index.ts';
	import type { Role } from '#lib/api/index.ts';
	import { getLocale, setLocale, t, type MessageKey } from '#lib/i18n/index.ts';
	import {
		platformNavigation,
		settingsNavigation,
		tabsForMode,
		type NavHref
	} from '#lib/navigation.ts';
	import { isSingleTenant } from '#lib/stores/instance.svelte.ts';
	import { buildBreadcrumb, type Crumb } from '#lib/breadcrumb.ts';
	import {
		forgetListQueries,
		getPageMeta,
		rememberListQuery
	} from '#lib/stores/page-meta.svelte.ts';
	import { clearSession, getSession } from '#lib/stores/session.svelte.ts';
	import { getTheme, setTheme } from '#lib/stores/theme.svelte.ts';
	import { Badge, DropdownMenu, Icon, ToastRegion } from './ui/index.ts';
	import type { MenuItem } from './ui/index.ts';
	import Brandmark from './Brandmark.svelte';
	import TenantSwitcher from './TenantSwitcher.svelte';

	let { children }: { children: Snippet } = $props();

	const session = $derived(getSession());
	const pathname = $derived(page.url.pathname);

	// Single mode has no platform screens (ADR-0024). The organization tabs need a current
	// organization; the platform tabs need the Platform Admin right (ADR-0029).
	const platformAdmin = $derived(!isSingleTenant() && (session?.platform_admin ?? false));
	const platformMode = $derived(
		platformAdmin && platformNavigation.items.some((item) => isActive(item.href))
	);
	const tenantTabs = $derived(
		session?.tenant ? tabsForMode('organization', session.role, platformAdmin) : []
	);
	const tabs = $derived(
		platformMode ? tabsForMode('platform', session?.role ?? null, platformAdmin) : tenantTabs
	);
	const settingsTab = $derived(tenantTabs.find((item) => item.href === '/(app)/settings'));

	const locale = $derived(getLocale());
	const theme = $derived(getTheme());

	const roleKeys: Record<Role, MessageKey> = {
		platform_admin: 'users.role.platformAdmin',
		admin: 'users.role.admin',
		operator: 'users.role.operator',
		viewer: 'users.role.viewer'
	};

	const displayName = $derived(session?.user?.display_name ?? t('topbar.account'));
	const accountSub = $derived(
		session?.tenant && session.role
			? `${t(roleKeys[session.role])} · ${session.tenant.name}`
			: t('topbar.noTenant')
	);
	const initials = $derived(
		displayName
			.split(/\s+/)
			.filter(Boolean)
			.slice(0, 2)
			.map((part) => part[0]?.toUpperCase() ?? '')
			.join('')
	);

	function isActive(href: NavHref): boolean {
		const target = resolve(href);
		return target === '/' ? pathname === '/' : pathname.startsWith(target);
	}

	const section = $derived.by<Crumb[]>(() => {
		const settingsItem = settingsNavigation
			.flatMap((group) => group.items)
			.find((item) => isActive(item.href));
		if (settingsItem) {
			return [
				{ label: t('nav.settings'), href: resolve('/(app)/settings') },
				{ label: t(settingsItem.labelKey), href: resolve(settingsItem.href) }
			];
		}
		const item = tabs.find((tab) => isActive(tab.href));
		return item ? [{ label: t(item.labelKey), href: resolve(item.href) }] : [];
	});

	const crumbs = $derived(buildBreadcrumb(section, getPageMeta()));

	$effect(() => rememberListQuery(page.url.pathname, page.url.search));

	// A list filter names objects of one organization, so it does not survive a switch.
	let shownTenant: string | undefined;
	$effect(() => {
		const tenant = session?.tenant?.id;
		if (shownTenant !== undefined && tenant !== shownTenant) forgetListQueries();
		shownTenant = tenant;
	});

	let prefersDark = $state(false);
	$effect(() => {
		const query = window.matchMedia('(prefers-color-scheme: dark)');
		prefersDark = query.matches;
		const update = (event: MediaQueryListEvent) => (prefersDark = event.matches);
		query.addEventListener('change', update);
		return () => query.removeEventListener('change', update);
	});
	const dark = $derived(theme === 'dark' || (theme === 'system' && prefersDark));

	function toggleTheme(): void {
		setTheme(dark ? 'light' : 'dark');
	}

	const accountItems = $derived<MenuItem[]>([
		...(settingsTab
			? [
					{
						label: t('nav.settings'),
						icon: 'settings' as const,
						onSelect: () => void goto(resolve('/(app)/settings'))
					}
				]
			: []),
		{
			label: t('topbar.theme.light'),
			group: 'theme',
			groupLabel: t('topbar.theme'),
			checked: theme === 'light',
			separatorBefore: !!settingsTab,
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
			groupLabel: t('topbar.language'),
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
			icon: 'arrow-right',
			tone: 'danger',
			separatorBefore: true,
			onSelect: signOut
		}
	]);

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
			<Brandmark />
			<span>{t('app.name')}</span>
		</a>

		<span class="topbar__divider"></span>

		<nav class="breadcrumb" aria-label={t('topbar.breadcrumb')}>
			{#if isSingleTenant()}
				<span class="breadcrumb__tenant">{session?.tenant?.name ?? t('topbar.noTenant')}</span>
			{:else}
				<TenantSwitcher {platformMode} />
			{/if}
			{#each crumbs as crumb, index (index)}
				<span class="breadcrumb__sep" aria-hidden="true">/</span>
				{#if index === crumbs.length - 1}
					<span aria-current="page">{crumb.label}</span>
				{:else if crumb.href}
					<!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- the levels hold resolved paths -->
					<a href={crumb.href}>{crumb.label}</a>
				{:else}
					<span>{crumb.label}</span>
				{/if}
			{/each}
		</nav>

		<div class="spacer"></div>

		<button
			type="button"
			class="icon-btn"
			aria-label={dark ? t('theme.switchToLight') : t('theme.switchToDark')}
			title={dark ? t('theme.switchToLight') : t('theme.switchToDark')}
			onclick={toggleTheme}
		>
			<Icon name={dark ? 'sun' : 'moon'} />
		</button>

		<DropdownMenu items={accountItems} align="end" menuClass="rh-menu--account">
			{#snippet header()}
				<p class="account-head__name">{displayName}</p>
				<p class="account-head__sub">{accountSub}</p>
				{#if session?.platform_admin}
					<!-- The Platform Admin right is an account right, not an organization (ADR-0029). -->
					<span class="account-head__right">
						<Badge tone="accent" icon="shield">{t('users.role.platformAdmin')}</Badge>
					</span>
				{/if}
			{/snippet}
			<span class="avatar" aria-hidden="true">{initials}</span>
			<span class="account__name">{displayName}</span>
			<Icon name="chevron-down" size="xs" />
		</DropdownMenu>
	</header>

	<nav class="tabs" aria-label={t('nav.primary')}>
		{#each tabs as item (item.href)}
			<a
				class="tab"
				href={resolve(item.href)}
				aria-current={isActive(item.href) ? 'page' : undefined}
			>
				<Icon name={item.icon} />
				{t(item.labelKey)}
			</a>
		{/each}
	</nav>

	<main id="main" class="page">
		<!-- A tenant switch remounts the screen, so its data reloads for the new tenant. -->
		{#key session?.tenant?.id}
			{@render children()}
		{/key}
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

	.topbar__divider {
		flex: none;
		width: 1px;
		height: 22px;
		background-color: var(--rh-color-border);
	}

	.breadcrumb {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
		min-width: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-base);
	}

	.breadcrumb__sep {
		color: var(--rh-color-text-subtle);
	}

	.breadcrumb a {
		color: var(--rh-color-text-muted);
	}

	.breadcrumb a:hover {
		color: var(--rh-color-text);
	}

	.breadcrumb [aria-current='page'] {
		color: var(--rh-color-text);
		font-weight: var(--rh-font-weight-semibold);
		white-space: nowrap;
	}

	.breadcrumb__tenant {
		color: var(--rh-color-text);
		font-weight: var(--rh-font-weight-semibold);
	}

	.spacer {
		flex: 1 1 auto;
	}

	.icon-btn {
		display: inline-flex;
		flex: none;
		align-items: center;
		justify-content: center;
		width: var(--rh-control-height);
		height: var(--rh-control-height);
		padding: 0;
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-control-bg);
		color: var(--rh-color-text);
		cursor: pointer;
	}

	.icon-btn:hover {
		border-color: var(--rh-color-border-strong);
		background-color: var(--rh-color-control-bg-hover);
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

	:global(.rh-menu--account) {
		min-width: 240px;
	}

	.account-head__name {
		margin: 0;
		font-weight: var(--rh-font-weight-semibold);
	}

	.account-head__sub {
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.account-head__right {
		display: flex;
		margin-top: var(--rh-space-2);
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
		scrollbar-width: none;
	}

	.tabs::-webkit-scrollbar {
		display: none;
	}

	.tab {
		display: inline-flex;
		flex: none;
		align-items: center;
		gap: var(--rh-space-2);
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

	.page {
		flex: 1;
		width: 100%;
		min-width: 0;
		max-width: var(--rh-layout-content-max-width);
		margin: 0 auto;
		padding: var(--rh-space-6) var(--rh-space-4) var(--rh-space-12);
	}

	@media (max-width: 640px) {
		.topbar {
			gap: var(--rh-space-2);
		}

		.breadcrumb [aria-current='page'],
		.breadcrumb__sep,
		.breadcrumb a {
			display: none;
		}

		.account__name {
			display: none;
		}

		.page {
			padding: var(--rh-space-4) var(--rh-space-3) var(--rh-space-8);
		}
	}
</style>
