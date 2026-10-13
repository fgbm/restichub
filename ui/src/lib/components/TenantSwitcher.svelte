<script lang="ts">
	/*
	 * The tenant switcher in the top bar (ADR-0025).
	 *
	 * The menu lists the memberships of the caller with the role and a check mark on the current
	 * tenant. It reloads the list on every open. A filter appears when the caller holds more than
	 * seven memberships. The list holds real organizations only; an account with no membership sees
	 * the empty state (ADR-0029).
	 *
	 * The keyboard moves with the arrows (`aria-activedescendant` names the active item), selects
	 * with Enter and closes with Escape; the focus returns to the button. A click outside closes
	 * the menu. A failed switch shows a toast; a lost session redirects to the sign-in screen.
	 */
	import { tick } from 'svelte';
	import { goto, invalidateAll } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ApiError, UnauthorizedError, service } from '#lib/api/index.ts';
	import type { Role, TenantAccess } from '#lib/api/index.ts';
	import { t, tErrorCode, type MessageKey } from '#lib/i18n/index.ts';
	import { clearSession, getSession, setSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';
	import Icon from './ui/Icon.svelte';

	let open = $state(false);
	let tenants = $state<TenantAccess[]>([]);
	let filter = $state('');
	let activeIndex = $state(0);

	let button = $state<HTMLButtonElement | null>(null);
	let menu = $state<HTMLDivElement | null>(null);
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

	// The list holds real organizations only (ADR-0029).
	const regular = $derived(tenants.filter((tenant) => matches(tenant)));
	const options = $derived(regular);
	const showFilter = $derived(tenants.length > 7);
	const activeId = $derived(options[activeIndex] ? itemId(options[activeIndex]) : undefined);

	function itemId(tenant: TenantAccess): string {
		return `tenant-switch-item-${tenant.id}`;
	}

	async function load(): Promise<void> {
		try {
			tenants = await service.listMemberships();
		} catch (error) {
			tenants = [];
			if (error instanceof UnauthorizedError) {
				clearSession();
				await goto(resolve('/(auth)/sign-in'));
			}
		}
	}

	async function openMenu(): Promise<void> {
		open = true;
		filter = '';
		// The list is reloaded on every open, so a membership added elsewhere shows up.
		await load();
		activeIndex = Math.max(
			0,
			tenants.findIndex((tenant) => tenant.id === currentId)
		);
		await tick();
		if (showFilter) filterInput?.focus();
		else menu?.focus();
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
				// Every screen reloads its data for the new tenant.
				await invalidateAll();
			} catch (error) {
				if (error instanceof UnauthorizedError) {
					clearSession();
					await goto(resolve('/(auth)/sign-in'));
					return;
				}
				const code = error instanceof ApiError ? error.code : undefined;
				pushToast('danger', tErrorCode(code) ?? t('alert.problemTitle'));
			}
		}
		close();
	}

	function onDocumentClick(event: MouseEvent): void {
		const target = event.target as Node | null;
		if (target && !menu?.contains(target) && !button?.contains(target)) {
			close();
		}
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

	$effect(() => {
		if (!open) return;
		document.addEventListener('click', onDocumentClick);
		return () => document.removeEventListener('click', onDocumentClick);
	});
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
		<Icon name="chevron-down" size="xs" />
	</button>

	{#if open}
		<div
			bind:this={menu}
			class="switcher__menu"
			role="menu"
			tabindex="-1"
			aria-label={t('topbar.switchTenant')}
			aria-activedescendant={activeId}
			onkeydown={onKeydown}
		>
			<p class="switcher__group-title">{t('topbar.yourTenants')}</p>
			{#if showFilter}
				<label class="switcher__filter">
					<Icon name="search" size="xs" />
					<input
						bind:this={filterInput}
						type="search"
						placeholder={t('topbar.filterTenants')}
						aria-label={t('topbar.filterTenants')}
						bind:value={filter}
					/>
				</label>
			{/if}
			{#if regular.length === 0}
				<p class="switcher__empty">{t('topbar.noOrganizations')}</p>
			{/if}
			{#each regular as tenant, index (tenant.id)}
				<button
					type="button"
					id={itemId(tenant)}
					class="switcher__item"
					class:switcher__item--active={index === activeIndex}
					role="menuitemradio"
					aria-checked={tenant.id === currentId}
					onmouseenter={() => (activeIndex = index)}
					onclick={() => select(tenant)}
				>
					<span class="switcher__item-name">{tenant.name}</span>
					<span class="switcher__item-role">{t(roleKeys[tenant.role])}</span>
					<span class="switcher__item-check">
						{#if tenant.id === currentId}<Icon name="check" size="xs" />{/if}
					</span>
				</button>
			{/each}
		</div>
	{/if}
</div>

<style>
	.switcher {
		position: relative;
		display: inline-flex;
	}

	.switcher__button {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-1);
		height: 26px;
		padding: 0 var(--rh-space-2);
		border: 0;
		border-radius: var(--rh-radius-sm);
		background: transparent;
		color: var(--rh-color-text);
		font: inherit;
		font-weight: var(--rh-font-weight-semibold);
		white-space: nowrap;
		cursor: pointer;
	}

	.switcher__button:hover,
	.switcher__button[aria-expanded='true'] {
		background-color: var(--rh-color-hover);
	}

	.switcher__menu {
		position: absolute;
		left: 0;
		top: calc(100% + 6px);
		min-width: 280px;
		max-height: 24rem;
		overflow-y: auto;
		padding: var(--rh-space-1);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-surface);
		box-shadow: var(--rh-shadow-menu);
		z-index: var(--rh-z-overlay);
	}

	.switcher__menu:focus-visible {
		outline: none;
	}

	.switcher__empty {
		margin: 0;
		padding: var(--rh-space-2);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.switcher__group-title {
		margin: 0;
		padding: var(--rh-space-1) var(--rh-space-2);
		color: var(--rh-color-text-subtle);
		font-size: var(--rh-font-size-xs);
		font-weight: var(--rh-font-weight-semibold);
	}

	.switcher__filter {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
		height: var(--rh-control-height);
		margin: var(--rh-space-1) var(--rh-space-2);
		padding: 0 var(--rh-space-2);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-input-bg);
		color: var(--rh-color-text-muted);
	}

	.switcher__filter:focus-within {
		border-color: var(--rh-color-focus);
		box-shadow: 0 0 0 3px var(--rh-color-focus-ring);
	}

	.switcher__filter input {
		flex: 1 1 auto;
		min-width: 0;
		border: 0;
		background: none;
		outline: 0;
		color: var(--rh-color-text);
		font-family: inherit;
		font-size: var(--rh-font-size-sm);
	}

	.switcher__filter input::placeholder {
		color: var(--rh-color-text-subtle);
	}

	.switcher__item {
		display: flex;
		align-items: center;
		gap: var(--rh-space-3);
		width: 100%;
		height: var(--rh-control-height);
		padding: 0 var(--rh-space-2);
		border: 0;
		border-radius: var(--rh-radius-sm);
		background: none;
		color: var(--rh-color-text);
		font-family: inherit;
		font-size: var(--rh-font-size-base);
		text-align: left;
		cursor: pointer;
	}

	.switcher__item--active,
	.switcher__item:hover {
		background-color: var(--rh-color-hover);
	}

	.switcher__item-name {
		flex: 1 1 auto;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.switcher__item-role {
		color: var(--rh-color-text-subtle);
		font-size: var(--rh-font-size-xs);
	}

	.switcher__item-check {
		display: inline-flex;
		justify-content: center;
		width: 16px;
		color: var(--rh-color-accent);
	}
</style>
