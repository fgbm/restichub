<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { ApiError, fieldError, namesField, service } from '#lib/api/index.ts';
	import type { ApiProblem, PlatformUser, Role, Tenant } from '#lib/api/index.ts';
	import PlatformUserDialogs from '#lib/components/PlatformUserDialogs.svelte';
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
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';
	import { activePlatformAdmins, platformChangeLock, platformResetLock } from '#lib/user-locks.ts';

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

	let dialogs = $state<PlatformUserDialogs>();
	const selfId = $derived(getSession()?.user?.id ?? null);
	const activeAdmins = $derived(activePlatformAdmins(accounts));

	const createFields = ['username', 'display_name', 'email', 'password'];

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
		const resetLock = platformResetLock(user, selfId);
		const changeLock = platformChangeLock(user, selfId, activeAdmins);
		return [
			{
				id: 'reset',
				label: t('users.resetPassword'),
				icon: 'key',
				disabled: Boolean(resetLock),
				hint: resetLock ?? undefined,
				onSelect: () => dialogs?.openReset(user)
			},
			{
				id: 'admin',
				label: user.platform_admin ? t('platformUsers.revokeAdmin') : t('platformUsers.grantAdmin'),
				icon: 'shield',
				disabled: Boolean(changeLock),
				hint: changeLock ?? undefined,
				onSelect: () => (user.platform_admin ? dialogs?.openRevoke(user) : dialogs?.openGrant(user))
			},
			{
				id: 'disable',
				label: user.disabled ? t('users.enableUser') : t('users.disableUser'),
				icon: 'power',
				danger: !user.disabled,
				disabled: Boolean(changeLock),
				hint: changeLock ?? undefined,
				onSelect: () => void dialogs?.toggleDisabled(user)
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
				email: newEmail.trim() === '' ? undefined : newEmail.trim(),
				password: newPassword,
				platform_admin: newPlatformAdmin,
				memberships: newTenantId ? [{ tenant_id: newTenantId, role: newRole }] : []
			});
			pushToast('success', t('platformUsers.created'));
			createOpen = false;
			await load();
		} catch (problem) {
			createProblem = problem instanceof ApiError ? problem.problem : null;
			createError = namesField(createProblem, createFields) ? null : messageOf(problem);
		} finally {
			creating = false;
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

	<ListView
		label={t('platformUsers.title')}
		items={shown}
		itemKey={(user) => user.id}
		icon={(user) => iconOf(userState(user))}
		pill={(user) => pillOf(userState(user))}
		title={(user) => user.display_name}
		href={(user) => resolve('/(app)/platform/users/[id]', { id: user.id })}
		{meta}
		actions={rowActions}
		moreLabel={(user) => t('users.moreActions', { name: user.display_name })}
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
		error={fieldError(createProblem, 'username')}
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
		error={fieldError(createProblem, 'email')}
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

<PlatformUserDialogs bind:this={dialogs} onchanged={load} />
