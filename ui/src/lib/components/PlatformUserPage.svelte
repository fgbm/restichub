<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { PlatformMembership, PlatformUser, Role, Tenant } from '#lib/api/index.ts';
	import PlatformUserDialogs from './PlatformUserDialogs.svelte';
	import { RecordPage, SummaryPanel } from './layout/index.ts';
	import {
		Button,
		ConfirmDialog,
		DataTable,
		DropdownMenu,
		Icon,
		Panel,
		Select,
		StatusPill
	} from './ui/index.ts';
	import type { KeyValueItem, MenuItem, RowAction, SelectOption, TableColumn } from './ui/index.ts';
	import { formatTime, relativeTime } from '#lib/format.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';
	import { activePlatformAdmins, platformChangeLock, platformResetLock } from '#lib/user-locks.ts';
	import { userState } from '#lib/vocabulary.ts';

	let { userId }: { userId: string } = $props();

	// The platform tenant is never a membership target, so these are the organization roles.
	const ASSIGNABLE: Role[] = ['admin', 'operator', 'viewer'];
	const ROLE_RANK: Record<Role, number> = { viewer: 0, operator: 1, admin: 2, platform_admin: 3 };

	let user = $state<PlatformUser | null>(null);
	let tenants = $state<Tenant[]>([]);
	let activeAdmins = $state(0);
	let loading = $state(true);
	let loadError = $state<string | null>(null);

	let dialogs = $state<PlatformUserDialogs>();

	// The role that each select shows, so a failed save returns the select to the saved role.
	let roleDrafts = $state<Record<string, Role>>({});
	let roleBusy = $state<string | null>(null);

	let addTenantId = $state('');
	let addRole = $state<Role>('viewer');
	let adding = $state(false);

	let removeTarget = $state<PlatformMembership | null>(null);
	let removeOpen = $state(false);
	let removing = $state(false);

	const selfId = $derived(getSession()?.user?.id ?? null);
	const self = $derived(user?.id === selfId);
	const name = $derived(user?.display_name ?? '');

	const roleLabels = $derived<Record<Role, string>>({
		platform_admin: t('users.role.platformAdmin'),
		admin: t('users.role.admin'),
		operator: t('users.role.operator'),
		viewer: t('users.role.viewer')
	});

	const roleOptions = $derived<SelectOption[]>(
		ASSIGNABLE.map((role) => ({ value: role, label: roleLabels[role] }))
	);

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	async function load(): Promise<void> {
		try {
			// The list gives the number of the active Platform Admins for the last-admin guard.
			const [account, tenantList, accounts] = await Promise.all([
				service.getPlatformUser(userId),
				service.listTenants(),
				service.listPlatformUsers()
			]);
			user = account;
			tenants = tenantList;
			activeAdmins = activePlatformAdmins(accounts);
			roleDrafts = Object.fromEntries(
				account.memberships.map((member) => [member.tenant_id, member.role])
			);
			loadError = null;
		} catch (problem) {
			loadError = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	onMount(load);

	const resetLock = $derived(user ? platformResetLock(user, selfId) : null);
	const changeLock = $derived(user ? platformChangeLock(user, selfId, activeAdmins) : null);

	const menuItems = $derived.by<MenuItem[]>(() => {
		const account = user;
		if (!account) return [];
		return [
			{
				label: account.platform_admin
					? t('platformUsers.revokeAdmin')
					: t('platformUsers.grantAdmin'),
				icon: 'shield',
				disabled: Boolean(changeLock),
				hint: changeLock ?? undefined,
				onSelect: () =>
					account.platform_admin ? dialogs?.openRevoke(account) : dialogs?.openGrant(account)
			},
			{
				label: account.disabled ? t('users.enableUser') : t('users.disableUser'),
				icon: 'power',
				tone: account.disabled ? 'default' : 'danger',
				separatorBefore: true,
				disabled: Boolean(changeLock),
				hint: changeLock ?? undefined,
				onSelect: () => void dialogs?.toggleDisabled(account)
			}
		];
	});

	const tenantById = $derived(new Map(tenants.map((tenant) => [tenant.id, tenant])));

	function membershipPill(member: PlatformMembership): string | null {
		if (tenantById.get(member.tenant_id)?.status === 'suspended') {
			return t('tenants.status.suspended');
		}
		return member.disabled ? t('users.status.disabled') : null;
	}

	const addOptions = $derived<SelectOption[]>(
		tenants
			.filter(
				(tenant) =>
					tenant.status === 'active' &&
					!user?.memberships.some((member) => member.tenant_id === tenant.id)
			)
			.sort((a, b) => a.name.localeCompare(b.name))
			.map((tenant) => ({ value: tenant.id, label: tenant.name }))
	);

	const columns = $derived<TableColumn<PlatformMembership>[]>([
		{
			key: 'tenant_name',
			header: t('platformUsers.organization'),
			sortable: false,
			render: organizationCell
		},
		{ key: 'role', header: t('platformUsers.role'), sortable: false, render: roleCell }
	]);

	function rowActions(member: PlatformMembership): RowAction[] {
		return [
			{
				id: 'remove',
				label: t('platformUsers.removeFrom', { name: member.tenant_name }),
				icon: 'trash',
				danger: true,
				disabled: self,
				hint: self ? t('platformUsers.removeSelfHint') : undefined,
				onSelect: () => openRemove(member)
			}
		];
	}

	/** The own role may only rise: a lower role in the current organization would outlive the session. */
	function roleOptionsOf(member: PlatformMembership): SelectOption[] {
		if (!self) return roleOptions;
		return roleOptions.filter(
			(option) => ROLE_RANK[option.value as Role] >= ROLE_RANK[member.role]
		);
	}

	async function changeRole(member: PlatformMembership, role: Role): Promise<void> {
		if (!user || role === member.role) return;
		roleDrafts[member.tenant_id] = role;
		roleBusy = member.tenant_id;
		try {
			await service.setPlatformMembershipRole(user.id, member.tenant_id, { role });
			member.role = role;
			pushToast('success', t('platformUsers.roleChanged'));
		} catch (problem) {
			roleDrafts[member.tenant_id] = member.role;
			pushToast('danger', messageOf(problem));
		} finally {
			roleBusy = null;
		}
	}

	async function addMembership(): Promise<void> {
		if (!user || !addTenantId) return;
		adding = true;
		try {
			await service.addPlatformMembership(user.id, { tenant_id: addTenantId, role: addRole });
			pushToast('success', t('platformUsers.added'));
			addTenantId = '';
			addRole = 'viewer';
			await load();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			adding = false;
		}
	}

	function openRemove(member: PlatformMembership): void {
		removeTarget = member;
		removeOpen = true;
	}

	async function confirmRemove(): Promise<void> {
		if (!user || !removeTarget) return;
		removing = true;
		try {
			await service.removePlatformMembership(user.id, removeTarget.tenant_id);
			pushToast('success', t('platformUsers.removed'));
			removeOpen = false;
			await load();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			removing = false;
		}
	}

	// `last_sign_in_at` comes with a later API change; until then the row stays hidden.
	const lastSignIn = $derived(
		(user as { last_sign_in_at?: string | null } | null)?.last_sign_in_at ?? null
	);

	const accountItems = $derived.by<KeyValueItem[]>(() => {
		if (!user) return [];
		const items: KeyValueItem[] = [
			{ key: 'username', label: t('users.username'), value: user.username, mono: true }
		];
		if (user.email) {
			items.push({
				key: 'email',
				label: t('platformUsers.email'),
				value: user.email,
				render: user.email_verified ? undefined : emailValue
			});
		}
		items.push(
			{ key: 'state', label: t('common.status'), value: userState(user).label },
			{
				key: 'admin',
				label: t('platformUsers.platformAdmin'),
				value: user.platform_admin ? t('common.yes') : t('common.no')
			}
		);
		if (lastSignIn) {
			items.push({
				key: 'last-sign-in',
				label: t('users.lastSignIn'),
				value: relativeTime(lastSignIn),
				render: lastSignInValue
			});
		}
		return items;
	});
</script>

{#snippet emailValue()}
	<span class="stack">
		{user?.email}
		<span class="muted">{t('users.emailUnverified')}</span>
	</span>
{/snippet}

{#snippet lastSignInValue()}
	{#if lastSignIn}
		<time datetime={lastSignIn} title={formatTime(lastSignIn)}>{relativeTime(lastSignIn)}</time>
	{/if}
{/snippet}

{#snippet organizationCell(member: PlatformMembership)}
	{@const pill = membershipPill(member)}
	<span class="org">
		<Icon name="building-2" />
		<strong class="org__name">{member.tenant_name}</strong>
		{#if pill}<StatusPill tone="neutral">{pill}</StatusPill>{/if}
	</span>
{/snippet}

{#snippet roleCell(member: PlatformMembership)}
	<Select
		class="role-select"
		aria-label={t('platformUsers.roleIn', { name: member.tenant_name })}
		title={self && member.role === 'admin' ? t('users.selfHint') : undefined}
		options={roleOptionsOf(member)}
		value={roleDrafts[member.tenant_id] ?? member.role}
		disabled={roleBusy !== null || (self && member.role === 'admin')}
		onchange={(event) =>
			changeRole(member, (event.currentTarget as HTMLSelectElement).value as Role)}
	/>
{/snippet}

{#snippet membershipsHead()}
	<h2 class="panel-title">
		{t('platformUsers.memberships')}
		<span class="panel-title__count">{user?.memberships.length ?? 0}</span>
	</h2>
{/snippet}

{#snippet addRow()}
	<div class="add">
		<Select
			id="membership-add-tenant"
			class={addTenantId ? 'add__org' : 'add__org add__org--empty'}
			label={t('platformUsers.organization')}
			placeholder={t('platformUsers.selectOrganization')}
			options={addOptions}
			bind:value={addTenantId}
		/>
		<Select
			id="membership-add-role"
			class="add__role"
			label={t('platformUsers.role')}
			options={roleOptions}
			bind:value={addRole}
		/>
		<Button icon="plus" loading={adding} disabled={!addTenantId} onclick={addMembership}>
			{t('platformUsers.addOrganization')}
		</Button>
	</div>
{/snippet}

<RecordPage title={name} {loading} error={loadError}>
	{#snippet status()}
		{#if user?.disabled}
			<StatusPill tone="neutral">{userState(user).label}</StatusPill>
		{/if}
	{/snippet}
	{#snippet actions()}
		{#if user}
			<Button
				size="sm"
				icon="key"
				disabled={Boolean(resetLock)}
				title={resetLock ?? undefined}
				onclick={() => user && dialogs?.openReset(user)}
			>
				{t('users.resetPassword')}
			</Button>
			<DropdownMenu
				icon="more"
				align="end"
				label={t('users.moreActions', { name })}
				items={menuItems}
			/>
		{/if}
	{/snippet}
	{#snippet side()}
		<SummaryPanel title={t('users.account')} items={accountItems} />
	{/snippet}

	{#if user}
		<div class="memberships">
			<Panel header={membershipsHead} flush footer={addOptions.length > 0 ? addRow : undefined}>
				<DataTable
					{columns}
					rows={user.memberships}
					rowKey={(member) => member.tenant_id}
					actions={rowActions}
					paginated={false}
					flush
					emptyTitle={t('platformUsers.noMemberships')}
					emptyDescription={user.platform_admin ? t('platformUsers.noMembershipsAdminHint') : ''}
				/>
			</Panel>
		</div>
	{/if}
</RecordPage>

<ConfirmDialog
	bind:open={removeOpen}
	icon="octagon"
	title={t('platformUsers.removeTitle')}
	message={t('platformUsers.removeBody', { name, org: removeTarget?.tenant_name ?? '' })}
	confirmLabel={t('platformUsers.removeConfirm')}
	tone="danger"
	loading={removing}
	onConfirm={confirmRemove}
/>

<PlatformUserDialogs bind:this={dialogs} onchanged={load} />

<style>
	.panel-title {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
		margin: 0;
		font-size: var(--rh-font-size-base);
		font-weight: var(--rh-font-weight-semibold);
		line-height: var(--rh-line-height-base);
	}

	.panel-title__count {
		min-width: 20px;
		padding: 0 6px;
		border-radius: var(--rh-radius-pill);
		background-color: var(--rh-color-neutral-bg);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-xs);
		line-height: 18px;
		text-align: center;
	}

	.org {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-2);
		min-width: 0;
		color: var(--rh-color-text-muted);
	}

	.org__name {
		color: var(--rh-color-text);
		font-weight: var(--rh-font-weight-semibold);
		overflow-wrap: anywhere;
	}

	.memberships :global(th:nth-child(2)),
	.memberships :global(td:nth-child(2)) {
		width: 200px;
	}

	.memberships :global(.role-select) {
		height: 28px;
		font-size: var(--rh-font-size-sm);
	}

	.add :global(.add__org--empty) {
		color: var(--rh-color-text-subtle);
	}

	.add {
		display: grid;
		grid-template-columns: minmax(0, 1fr) 200px auto;
		gap: var(--rh-space-2);
		align-items: end;
		width: 100%;
		padding: var(--rh-space-1) 0;
	}

	.stack {
		display: flex;
		flex-direction: column;
	}

	.muted {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	@media (max-width: 720px) {
		.memberships :global(th:nth-child(2)),
		.memberships :global(td:nth-child(2)) {
			width: 1%;
		}

		/* The select takes the width of the longest role, so no role name is cut. */
		.memberships :global(.role-select) {
			width: auto;
		}

		.add {
			grid-template-columns: minmax(0, 1fr);
		}
	}
</style>
