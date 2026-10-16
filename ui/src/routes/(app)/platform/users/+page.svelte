<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, fieldError, namesField, service } from '#lib/api/index.ts';
	import type { ApiProblem, PlatformUser, Role, Tenant } from '#lib/api/index.ts';
	import { Page } from '#lib/components/layout/index.ts';
	import {
		Alert,
		Button,
		Checkbox,
		Dialog,
		FilterBar,
		ListView,
		PasswordField,
		Select,
		TextField
	} from '#lib/components/ui/index.ts';
	import type { FilterFacet, ListMeta, RowAction, SelectOption } from '#lib/components/ui/index.ts';
	import { plural } from '#lib/list-format.ts';
	import { iconOf, pillOf, userState } from '#lib/vocabulary.ts';
	import { facetCounts, matchesSearch, splitList } from '#lib/filters.ts';
	import { queryFilters } from '#lib/stores/query-filters.svelte.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	// The platform tenant holds the role `platform_admin` only, so a membership here is below it.
	const ASSIGNABLE: Role[] = ['admin', 'operator', 'viewer'];

	let accounts = $state<PlatformUser[]>([]);
	let tenants = $state<Tenant[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let createOpen = $state(false);
	let creating = $state(false);
	let createError = $state<string | null>(null);
	let createProblem = $state<ApiProblem | null>(null);
	let newUsername = $state('');
	let newDisplayName = $state('');
	let newEmail = $state('');
	let newPassword = $state('');
	let newTenantId = $state('');
	let newRole = $state<Role>('viewer');
	let newPlatformAdmin = $state(false);

	let manageOpen = $state(false);
	let manageUser = $state<PlatformUser | null>(null);
	let manageTenantId = $state('');
	let manageRole = $state<Role>('viewer');
	let busyKey = $state<string | null>(null);

	// A grant of the right asks for a new password: the old password may be known to a tenant Admin
	// (ADR-0029).
	let grantOpen = $state(false);
	let grantUser = $state<PlatformUser | null>(null);
	let grantPassword = $state('');
	let granting = $state(false);
	let grantError = $state<string | null>(null);

	const createFields = ['username', 'display_name', 'email', 'password'];
	const emailTaken = $derived(
		createProblem?.code === 'email_taken' || createProblem?.code === 'username_taken'
	);

	const roleKeys = $derived<Record<Role, string>>({
		platform_admin: t('users.role.platformAdmin'),
		admin: t('users.role.admin'),
		operator: t('users.role.operator'),
		viewer: t('users.role.viewer')
	});

	const roleOptions = $derived<SelectOption[]>(
		ASSIGNABLE.map((role) => ({ value: role, label: roleKeys[role] }))
	);

	const tenantOptions = $derived<SelectOption[]>([
		{ value: '', label: t('platformUsers.noOrganizations') },
		...tenants
			.filter((tenant) => tenant.status === 'active')
			.map((tenant) => ({ value: tenant.id, label: tenant.name }))
	]);

	const filters = queryFilters(['q', 'tenant', 'role', 'state', 'sort', 'order'] as const);

	function rolesOf(user: PlatformUser): Role[] {
		return [
			...(user.platform_admin ? (['platform_admin'] as Role[]) : []),
			...user.memberships.map((member) => member.role)
		];
	}

	const facets = $derived<FilterFacet[]>([
		{
			key: 'tenant',
			label: t('platformUsers.facet.organization'),
			options: facetCounts(accounts, (user) => user.memberships.map((member) => member.tenant_name))
		},
		{
			key: 'role',
			label: t('users.role'),
			options: facetCounts(accounts, (user) => [...new Set(rolesOf(user))]).map((option) => ({
				...option,
				label: roleKeys[option.value as Role] ?? option.value
			}))
		}
	]);

	const matching = $derived(
		accounts.filter((user) => {
			const { q, tenant, role } = filters.value;
			const names = user.memberships.map((member) => member.tenant_name);
			if (tenant && !splitList(tenant).some((name) => names.includes(name))) return false;
			if (role && !splitList(role).some((item) => rolesOf(user).includes(item as Role))) {
				return false;
			}
			return matchesSearch(q, [user.username, user.display_name, user.email]);
		})
	);

	const states = $derived({
		default: 'active',
		options: [
			{
				value: 'active',
				label: t('users.tab.active'),
				count: matching.filter((user) => !user.disabled).length
			},
			{
				value: 'disabled',
				label: t('users.tab.disabled'),
				count: matching.filter((user) => user.disabled).length
			}
		]
	});

	const sort = $derived({
		default: 'name',
		options: [{ value: 'name', label: t('users.sort.name') }]
	});

	const shown = $derived.by(() => {
		const disabled = filters.value.state === 'disabled';
		const factor = filters.value.order === 'desc' ? -1 : 1;
		return matching
			.filter((user) => user.disabled === disabled)
			.sort((a, b) => factor * a.display_name.localeCompare(b.display_name));
	});

	const filtered = $derived(
		accounts.length > 0 && Boolean(filters.value.q || filters.value.tenant || filters.value.role)
	);

	function resetFilters(): void {
		filters.update({ q: '', tenant: '', role: '', state: '' });
	}

	// Two memberships read as text; more read as a count.
	function membershipsText(user: PlatformUser): string {
		if (user.memberships.length === 0) return t('platformUsers.noOrganizations');
		if (user.memberships.length > 2)
			return plural('platformUsers.orgCount', user.memberships.length);
		return user.memberships
			.map((member) =>
				t('platformUsers.meta.membership', {
					name: member.tenant_name,
					role: roleKeys[member.role].toLocaleLowerCase()
				})
			)
			.join(', ');
	}

	function meta(user: PlatformUser): ListMeta[] {
		return [
			{ text: user.username, mono: true },
			membershipsText(user),
			user.platform_admin ? t('platformUsers.meta.platformAdmin') : null,
			user.email
				? user.email_verified
					? user.email
					: t('platformUsers.meta.unverified', { email: user.email })
				: null
		];
	}

	function rowActions(user: PlatformUser): RowAction[] {
		return [
			{
				id: 'memberships',
				label: t('platformUsers.manage'),
				icon: 'building-2',
				onSelect: () => openManage(user)
			},
			{
				id: 'admin',
				label: user.platform_admin ? t('platformUsers.revokeAdmin') : t('platformUsers.grantAdmin'),
				icon: 'shield',
				disabled: busyKey !== null,
				onSelect: () => (user.platform_admin ? revokeRight(user) : openGrant(user))
			},
			{
				id: 'disable',
				label: user.disabled ? t('users.enableUser') : t('users.disableUser'),
				icon: 'power',
				danger: !user.disabled,
				disabled: busyKey !== null,
				onSelect: () => toggleDisabled(user)
			}
		];
	}

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			[accounts, tenants] = await Promise.all([service.listPlatformUsers(), service.listTenants()]);
		} catch (problem) {
			error = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	onMount(load);

	function openCreate(): void {
		newUsername = '';
		newDisplayName = '';
		newEmail = '';
		newPassword = '';
		newTenantId = '';
		newRole = 'viewer';
		newPlatformAdmin = false;
		createError = null;
		createProblem = null;
		createOpen = true;
	}

	async function createUser(): Promise<void> {
		if (!newUsername || !newDisplayName || newPassword.length < 12) return;
		creating = true;
		createError = null;
		createProblem = null;
		try {
			await service.createPlatformUser({
				username: newUsername,
				display_name: newDisplayName,
				email: newEmail.trim() === '' ? null : newEmail,
				password: newPassword,
				platform_admin: newPlatformAdmin,
				memberships: newTenantId ? [{ tenant_id: newTenantId, role: newRole }] : []
			});
			pushToast('success', t('platformUsers.created'));
			createOpen = false;
			await load();
		} catch (problem) {
			createProblem = problem instanceof ApiError ? problem.problem : null;
			createError =
				emailTaken || namesField(createProblem, createFields) ? null : messageOf(problem);
		} finally {
			creating = false;
		}
	}

	function openManage(user: PlatformUser): void {
		manageUser = user;
		manageTenantId = '';
		manageRole = 'viewer';
		manageOpen = true;
	}

	async function addMembership(): Promise<void> {
		if (!manageUser || !manageTenantId) return;
		busyKey = 'add';
		try {
			await service.addPlatformMembership(manageUser.id, {
				tenant_id: manageTenantId,
				role: manageRole
			});
			pushToast('success', t('platformUsers.added'));
			await load();
			refreshManage();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			busyKey = null;
		}
	}

	async function changeRole(tenantId: string, role: Role): Promise<void> {
		if (!manageUser) return;
		busyKey = `role:${tenantId}`;
		try {
			await service.setPlatformMembershipRole(manageUser.id, tenantId, { role });
			pushToast('success', t('platformUsers.roleChanged'));
			await load();
			refreshManage();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			busyKey = null;
		}
	}

	async function removeMembership(tenantId: string): Promise<void> {
		if (!manageUser) return;
		busyKey = `remove:${tenantId}`;
		try {
			await service.removePlatformMembership(manageUser.id, tenantId);
			pushToast('success', t('platformUsers.removed'));
			await load();
			refreshManage();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			busyKey = null;
		}
	}

	function refreshManage(): void {
		if (!manageUser) return;
		manageUser = accounts.find((account) => account.id === manageUser?.id) ?? null;
	}

	function openGrant(user: PlatformUser): void {
		grantUser = user;
		grantPassword = '';
		grantError = null;
		grantOpen = true;
	}

	async function grantRight(): Promise<void> {
		if (!grantUser || grantPassword.length < 12) return;
		granting = true;
		grantError = null;
		try {
			await service.updatePlatformUser(grantUser.id, {
				platform_admin: true,
				password: grantPassword
			});
			pushToast('success', t('platformUsers.rightGranted'));
			grantOpen = false;
			await load();
		} catch (problem) {
			grantError = messageOf(problem);
		} finally {
			granting = false;
		}
	}

	async function revokeRight(user: PlatformUser): Promise<void> {
		busyKey = `right:${user.id}`;
		try {
			await service.updatePlatformUser(user.id, { platform_admin: false });
			pushToast('success', t('platformUsers.rightRemoved'));
			await load();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			busyKey = null;
		}
	}

	async function toggleDisabled(user: PlatformUser): Promise<void> {
		busyKey = `disable:${user.id}`;
		try {
			await service.updatePlatformUser(user.id, { disabled: !user.disabled });
			pushToast('success', user.disabled ? t('users.enabledToast') : t('users.disabledToast'));
			await load();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			busyKey = null;
		}
	}
</script>

<Page
	title={t('platformUsers.title')}
	documentTitle={`${t('platformUsers.title')} · ${t('nav.platformMode')}`}
	description={t('platformUsers.description')}
	count={loading ? undefined : accounts.length}
>
	{#snippet actions()}
		<Button variant="primary" icon="plus" onclick={openCreate}>{t('platformUsers.create')}</Button>
	{/snippet}

	<!-- An account has no record page: the row opens its memberships (design/style/ui-principles.md, 2.2). -->
	<!-- An account has no record page: the name opens its memberships. -->
	<ListView
		label={t('platformUsers.title')}
		items={shown}
		itemKey={(user) => user.id}
		icon={(user) => iconOf(userState(user))}
		pill={(user) => pillOf(userState(user))}
		title={(user) => user.display_name}
		onOpen={openManage}
		{meta}
		actions={rowActions}
		moreLabel={(user) => t('users.moreActions', { name: user.username })}
		{loading}
		{error}
		{filtered}
		onReset={resetFilters}
		emptyIcon="users"
		emptyTitle={t('users.emptyTitle')}
	>
		{#snippet toolbar()}
			<FilterBar
				label={t('users.filters')}
				value={filters.value}
				search={{ key: 'q', label: t('hosts.search'), placeholder: t('users.searchPlaceholder') }}
				{states}
				{facets}
				{sort}
				onchange={filters.update}
				onreset={resetFilters}
			/>
		{/snippet}
	</ListView>
</Page>

<Dialog bind:open={createOpen} title={t('platformUsers.createTitle')} onsubmit={createUser}>
	{#if createError}
		<Alert variant="inline" tone="danger" title={createError} />
	{/if}
	<TextField
		id="platform-user-username"
		label={t('platformUsers.username')}
		error={emailTaken ? tErrorCode('username_taken') : fieldError(createProblem, 'username')}
		bind:value={newUsername}
		mono
		autocomplete="off"
		required
	/>
	<TextField
		id="platform-user-display-name"
		label={t('platformUsers.displayName')}
		error={fieldError(createProblem, 'display_name')}
		bind:value={newDisplayName}
		autocomplete="off"
		required
	/>
	<TextField
		id="platform-user-email"
		label={t('platformUsers.email')}
		help={t('platformUsers.emailHint')}
		error={emailTaken ? tErrorCode('email_taken') : fieldError(createProblem, 'email')}
		bind:value={newEmail}
		type="email"
		autocomplete="off"
	/>
	<PasswordField
		id="platform-user-password"
		label={t('users.initialPassword')}
		help={t('setup.passwordHint')}
		error={fieldError(createProblem, 'password')}
		bind:value={newPassword}
		autocomplete="new-password"
		required
	/>
	<Checkbox
		id="platform-user-admin"
		label={t('platformUsers.platformAdmin')}
		bind:checked={newPlatformAdmin}
	/>
	<Select
		id="platform-user-tenant"
		label={t('platformUsers.organization')}
		bind:value={newTenantId}
		options={tenantOptions}
	/>
	{#if newTenantId}
		<Select
			id="platform-user-role"
			label={t('platformUsers.role')}
			bind:value={newRole}
			options={roleOptions}
		/>
	{/if}
	{#snippet footer()}
		<Button onclick={() => (createOpen = false)}>{t('common.cancel')}</Button>
		<Button
			type="submit"
			variant="primary"
			loading={creating}
			disabled={!newUsername || !newDisplayName || newPassword.length < 12}
		>
			{t('platformUsers.create')}
		</Button>
	{/snippet}
</Dialog>

<Dialog bind:open={manageOpen} size="lg" icon="users" title={manageUser?.username ?? ''}>
	{#if manageUser}
		<h3>{t('platformUsers.memberships')}</h3>
		{#if manageUser.memberships.length === 0}
			<p>{t('platformUsers.noOrganizations')}</p>
		{:else}
			{#each manageUser.memberships as member (member.tenant_id)}
				<div class="membership">
					<span class="membership__name">{member.tenant_name}</span>
					<Select
						id={`membership-role-${member.tenant_id}`}
						label={t('platformUsers.role')}
						value={member.role}
						options={roleOptions}
						onchange={(event) =>
							changeRole(
								member.tenant_id,
								(event.currentTarget as HTMLSelectElement).value as Role
							)}
					/>
					<Button
						size="sm"
						variant="ghost"
						loading={busyKey === `remove:${member.tenant_id}`}
						onclick={() => removeMembership(member.tenant_id)}
					>
						{t('platformUsers.removeMembership')}
					</Button>
				</div>
			{/each}
		{/if}
		<h3>{t('platformUsers.addOrganization')}</h3>
		<Select
			id="manage-tenant"
			label={t('platformUsers.organization')}
			bind:value={manageTenantId}
			options={tenantOptions}
		/>
		{#if manageTenantId}
			<Select
				id="manage-role"
				label={t('platformUsers.role')}
				bind:value={manageRole}
				options={roleOptions}
			/>
			<Button
				variant="primary"
				loading={busyKey === 'add'}
				disabled={!manageTenantId}
				onclick={addMembership}
			>
				{t('platformUsers.addOrganization')}
			</Button>
		{/if}
	{/if}
</Dialog>

<Dialog
	bind:open={grantOpen}
	icon="shield"
	title={t('platformUsers.grantTitle')}
	onsubmit={grantRight}
>
	{#if grantError}
		<Alert variant="inline" tone="danger" title={grantError} />
	{/if}
	<p>{t('platformUsers.grantHint', { name: grantUser?.username ?? '' })}</p>
	<PasswordField
		id="platform-user-grant-password"
		label={t('setup.password')}
		help={t('setup.passwordHint')}
		bind:value={grantPassword}
		autocomplete="new-password"
		required
	/>
	{#snippet footer()}
		<Button onclick={() => (grantOpen = false)}>{t('common.cancel')}</Button>
		<Button type="submit" variant="primary" loading={granting} disabled={grantPassword.length < 12}>
			{t('platformUsers.grantAdmin')}
		</Button>
	{/snippet}
</Dialog>

<style>
	.membership {
		display: flex;
		align-items: flex-end;
		gap: var(--rh-space-2);
	}

	.membership__name {
		flex: 1;
	}
</style>
