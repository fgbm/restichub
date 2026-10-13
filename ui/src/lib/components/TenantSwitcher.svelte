<script lang="ts">
	/*
	 * The tenant switcher in the top bar (ADR-0025).
	 *
	 * The menu lists the memberships of the caller with the role and a check mark on the current
	 * tenant. A filter appears when the caller holds more than seven memberships. A Platform Admin
	 * gets a separate Platform entry. The keyboard moves with the arrows, selects with Enter and
	 * closes with Escape; the focus returns to the button.
	 */
	import { tick } from 'svelte';
	import { service } from '#lib/api/index.ts';
	import type { Role, TenantAccess } from '#lib/api/index.ts';
	import { t, type MessageKey } from '#lib/i18n/index.ts';
	import { getSession, setSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	let open = $state(false);
	let tenants = $state<TenantAccess[]>([]);
	let filter = $state('');
	let activeIndex = $state(0);
	let loaded = $state(false);

	let button = $state<HTMLButtonElement | null>(null);
	let filterInput = $state<HTMLInputElement | null>(null);

	const roleKeys = $derived<Record<Role, MessageKey>>({
		platform_admin: 'users.role.platformAdmin',
		admin: 'users.role.admin',
		operator: 'users.role.operator',
		viewer: 'users.role.viewer'
	});

	const session = $derived(getSession());
	const currentId = $derived(session?.tenant?.id ?? '');
	const query = $derived(filter.trim().toLowerCase());

	function matches(tenant: TenantAccess): boolean {
		return tenant.name.toLowerCase().includes(query);
	}

	const regular = $derived(
		tenants.filter((tenant) => tenant.role !== 'platform_admin' && matches(tenant))
	);
	const platform = $derived(
		tenants.filter((tenant) => tenant.role === 'platform_admin' && matches(tenant))
	);
	const options = $derived([...regular, ...platform]);
	const showFilter = $derived(tenants.length > 7);

	async function load(): Promise<void> {
		try {
			tenants = await service.listMemberships();
		} catch {
			tenants = [];
		}
		loaded = true;
	}

	async function openMenu(): Promise<void> {
		open = true;
		filter = '';
		activeIndex = Math.max(
			0,
			tenants.findIndex((tenant) => tenant.id === currentId)
		);
		if (!loaded) await load();
		await tick();
		if (showFilter) filterInput?.focus();
	}

	function close(): void {
		if (!open) return;
		open = false;
		button?.focus();
	}

	async function toggle(): Promise<void> {
		if (open) close();
		else await openMenu();
	}

	async function select(tenant: TenantAccess): Promise<void> {
		if (tenant.id !== currentId) {
			try {
				const me = await service.switchTenant(tenant.id);
				setSession(me);
				pushToast('success', t('topbar.switched', { name: tenant.name }));
			} catch {
				// The switch failed; the session stays on the current tenant.
			}
		}
		close();
	}

	function onKeydown(event: KeyboardEvent): void {
		if (event.key === 'Escape') {
			event.preventDefault();
			close();
			return;
		}
		if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
			event.preventDefault();
			if (!open) {
				void toggle();
				return;
			}
			if (options.length > 0) {
				const delta = event.key === 'ArrowDown' ? 1 : -1;
				activeIndex = (activeIndex + delta + options.length) % options.length;
			}
			return;
		}
		if (event.key === 'Enter' && open) {
			const tenant = options[activeIndex];
			if (tenant) {
				event.preventDefault();
				void select(tenant);
			}
		}
	}
</script>

<div class="switcher">
	<button
		bind:this={button}
		type="button"
		class="switcher__button"
		aria-haspopup="menu"
		aria-expanded={open}
		aria-label={t('topbar.switchTenant')}
		onclick={toggle}
		onkeydown={onKeydown}
	>
		<span class="switcher__name">{session?.tenant?.name ?? t('topbar.noTenant')}</span>
		<span class="switcher__caret" aria-hidden="true">▾</span>
	</button>

	{#if open}
		<div
			class="switcher__menu"
			role="menu"
			tabindex="-1"
			aria-label={t('topbar.switchTenant')}
			onkeydown={onKeydown}
		>
			<p class="switcher__group-title">{t('topbar.yourTenants')}</p>
			{#if showFilter}
				<input
					bind:this={filterInput}
					class="switcher__filter"
					type="search"
					placeholder={t('topbar.filterTenants')}
					aria-label={t('topbar.filterTenants')}
					bind:value={filter}
				/>
			{/if}
			{#each regular as tenant, index (tenant.id)}
				<button
					type="button"
					class="switcher__item"
					class:switcher__item--active={index === activeIndex}
					role="menuitemradio"
					aria-checked={tenant.id === currentId}
					onmouseenter={() => (activeIndex = index)}
					onclick={() => select(tenant)}
				>
					<span class="switcher__item-name">{tenant.name}</span>
					<span class="switcher__item-role">{t(roleKeys[tenant.role])}</span>
					<span class="switcher__item-check" aria-hidden="true">
						{tenant.id === currentId ? '✓' : ''}
					</span>
				</button>
			{/each}
			{#if platform.length > 0}
				<p class="switcher__group-title">{t('topbar.platform')}</p>
				{#each platform as tenant, index (tenant.id)}
					{@const position = regular.length + index}
					<button
						type="button"
						class="switcher__item"
						class:switcher__item--active={position === activeIndex}
						role="menuitemradio"
						aria-checked={tenant.id === currentId}
						onmouseenter={() => (activeIndex = position)}
						onclick={() => select(tenant)}
					>
						<span class="switcher__item-name">{t('topbar.platform')}</span>
						<span class="switcher__item-role">{t(roleKeys[tenant.role])}</span>
						<span class="switcher__item-check" aria-hidden="true">
							{tenant.id === currentId ? '✓' : ''}
						</span>
					</button>
				{/each}
			{/if}
		</div>
	{/if}
</div>

<style>
	.switcher {
		position: relative;
	}

	.switcher__button {
		display: flex;
		align-items: center;
		gap: var(--rh-space-1);
		padding: var(--rh-space-1) var(--rh-space-2);
		border: var(--rh-border-width) solid transparent;
		border-radius: var(--rh-radius-sm);
		background: transparent;
		color: var(--rh-color-text);
		font-size: var(--rh-font-size-sm);
		cursor: pointer;
	}

	.switcher__button:hover {
		background-color: var(--rh-color-hover);
	}

	.switcher__name {
		font-weight: var(--rh-font-weight-medium);
	}

	.switcher__menu {
		position: absolute;
		left: 0;
		top: calc(100% + var(--rh-space-1));
		min-width: 16rem;
		max-height: 24rem;
		overflow-y: auto;
		padding: var(--rh-space-2);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-surface);
		box-shadow: var(--rh-shadow-overlay);
		z-index: var(--rh-z-topbar);
	}

	.switcher__group-title {
		margin: var(--rh-space-2) var(--rh-space-2) var(--rh-space-1);
		font-size: var(--rh-font-size-xs);
		text-transform: uppercase;
		letter-spacing: 0.04em;
		color: var(--rh-color-text-subtle);
	}

	.switcher__filter {
		width: 100%;
		margin-bottom: var(--rh-space-1);
		padding: var(--rh-space-1) var(--rh-space-2);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-sm);
		background-color: var(--rh-color-surface);
		color: var(--rh-color-text);
		font-size: var(--rh-font-size-sm);
	}

	.switcher__item {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
		width: 100%;
		padding: var(--rh-space-1) var(--rh-space-2);
		border: 0;
		border-radius: var(--rh-radius-sm);
		background: transparent;
		color: var(--rh-color-text);
		font-size: var(--rh-font-size-sm);
		text-align: left;
		cursor: pointer;
	}

	.switcher__item--active,
	.switcher__item:hover {
		background-color: var(--rh-color-hover);
	}

	.switcher__item-name {
		flex: 1;
	}

	.switcher__item-role {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-xs);
	}

	.switcher__item-check {
		width: 1ch;
		color: var(--rh-color-accent);
	}
</style>
