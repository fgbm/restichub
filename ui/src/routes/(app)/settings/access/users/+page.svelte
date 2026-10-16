<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, assignableRoles, fieldError, namesField, service } from '#lib/api/index.ts';
	import type { ApiProblem, Role, User } from '#lib/api/index.ts';
	import { SettingsLayout } from '#lib/components/index.ts';
	import {
		Alert,
		Button,
		Dialog,
		Icon,
		KeyValue,
		ListView,
		Panel,
		PasswordField,
		FilterBar,
		Select,
		SidePanel,
		TextField
	} from '#lib/components/ui/index.ts';
	import type {
		FilterFacet,
		IconName,
		KeyValueItem,
		ListMeta,
		RowAction,
		SelectOption
	} from '#lib/components/ui/index.ts';
	import { iconOf, pillOf, userState } from '#lib/vocabulary.ts';
	import { matchesSearch, splitList } from '#lib/filters.ts';
	import { queryFilters } from '#lib/stores/query-filters.svelte.ts';
	import { t, tErrorCode, type MessageKey } from '#lib/i18n/index.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	let users = $state<User[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let createOpen = $state(false);
	let creating = $state(false);
	let createError = $state<string | null>(null);
	let createProblem = $state<ApiProblem | null>(null);

	let resetOpen = $state(false);
	let resetUser = $state<User | null>(null);
	let resetting = $state(false);
	let resetError = $state<string | null>(null);
	let newResetPassword = $state('');

	// The fields of the create form. A problem of another field (for example `body`) shows the
	// general alert instead of the message next to an input.
	const createFields = ['username', 'display_name', 'role', 'password'];

	// The create endpoint answers HTTP 409 `username_taken` without a field name.
	const usernameTaken = $derived(createProblem?.code === 'username_taken');

	let newUsername = $state('');
	let newDisplayName = $state('');
	let newRole = $state<Role>('viewer');
	let newPassword = $state('');

	const tenantName = $derived(getSession()?.tenant?.name ?? '');

	const roleKeys = $derived<Record<Role, string>>({
		platform_admin: t('users.role.platformAdmin'),
		admin: t('users.role.admin'),
		operator: t('users.role.operator'),
		viewer: t('users.role.viewer')
	});

	// The least privilege comes first and is the default.
	const roles = $derived([...assignableRoles()].reverse());

	const roleOptions = $derived<SelectOption[]>(
		roles.map((role) => ({
			value: role,
			label: roleKeys[role]
		}))
	);

	const roleHelp: { role: Role; icon: IconName; body: MessageKey }[] = [
		{ role: 'admin', icon: 'shield', body: 'users.roles.admin' },
		{ role: 'operator', icon: 'shield', body: 'users.roles.operator' },
		{ role: 'viewer', icon: 'eye', body: 'users.roles.viewer' }
	];

	const filters = queryFilters(['q', 'role', 'state', 'sort', 'order'] as const);

	const facets = $derived<FilterFacet[]>([
		{
			key: 'role',
			label: t('users.role'),
			options: (Object.keys(roleKeys) as Role[])
				.filter((role) => users.some((user) => user.role === role))
				.map((role) => ({
					value: role,
					label: roleKeys[role],
					count: users.filter((user) => user.role === role).length
				}))
		}
	]);

	const matching = $derived(
		users.filter((user) => {
			const { q, role } = filters.value;
			if (role && !splitList(role).includes(user.role)) return false;
			return matchesSearch(q, [user.display_name, user.username]);
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

	const ROLE_ORDER: Role[] = ['platform_admin', 'admin', 'operator', 'viewer'];
	const sort = $derived({
		default: 'name',
		options: [
			{ value: 'name', label: t('users.sort.name') },
			{ value: 'role', label: t('users.sort.role') }
		]
	});

	const shown = $derived.by(() => {
		const disabled = filters.value.state === 'disabled';
		const list = matching.filter((user) => user.disabled === disabled);
		const factor = filters.value.order === 'desc' ? -1 : 1;
		const byRole = filters.value.sort === 'role';
		return [...list].sort(
			(a, b) =>
				factor *
				(byRole
					? ROLE_ORDER.indexOf(a.role) - ROLE_ORDER.indexOf(b.role) ||
						a.display_name.localeCompare(b.display_name)
					: a.display_name.localeCompare(b.display_name))
		);
	});

	const filtered = $derived(users.length > 0 && Boolean(filters.value.q || filters.value.role));

	function resetFilters(): void {
		filters.update({ q: '', role: '', state: '' });
	}

	function meta(user: User): ListMeta[] {
		return [
			{ text: user.username, mono: true },
			roleKeys[user.role],
			user.shared ? t('users.meta.shared') : null
		];
	}

	let detail = $state<User | null>(null);
	let detailOpen = $state(false);

	function openDetail(user: User): void {
		detail = user;
		detailOpen = true;
	}

	const detailItems = $derived<KeyValueItem[]>(
		detail
			? [
					{ key: 'username', label: t('users.username'), value: detail.username, mono: true },
					{ key: 'role', label: t('users.role'), value: roleKeys[detail.role] },
					{ key: 'state', label: t('common.status'), value: userState(detail).label },
					...(detail.shared
						? [{ key: 'shared', label: t('users.shared'), value: t('common.yes') }]
						: [])
				]
			: []
	);

	let roleOpen = $state(false);
	let roleUser = $state<User | null>(null);
	let roleValue = $state<Role>('viewer');
	let roleSaving = $state(false);
	let roleError = $state<string | null>(null);

	function openRole(user: User): void {
		roleUser = user;
		roleValue = user.role;
		roleError = null;
		roleOpen = true;
	}

	async function saveRole(): Promise<void> {
		if (!roleUser) return;
		roleSaving = true;
		roleError = null;
		try {
			await service.updateUser(roleUser.id, { role: roleValue });
			pushToast('success', t('users.roleChanged'));
			roleOpen = false;
			await load();
		} catch (problem) {
			roleError = messageOf(problem);
		} finally {
			roleSaving = false;
		}
	}

	function rowActions(user: User): RowAction[] {
		return [
			{
				id: 'role',
				label: t('users.changeRole'),
				icon: 'shield',
				onSelect: () => openRole(user)
			},
			{
				id: 'reset',
				label: t('users.resetPassword'),
				hint: user.shared ? t('users.sharedResetHint') : undefined,
				icon: 'key',
				disabled: user.shared,
				onSelect: () => openReset(user)
			},
			{
				id: 'disable',
				label: user.disabled ? t('users.enableUser') : t('users.disableUser'),
				icon: 'power',
				danger: !user.disabled,
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
			users = await service.listUsers();
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
		newRole = roles[0];
		newPassword = '';
		createError = null;
		createProblem = null;
		createOpen = true;
	}

	const canCreate = $derived(!!newUsername && !!newDisplayName && newPassword.length >= 12);

	async function createUser(): Promise<void> {
		if (!canCreate) return;
		creating = true;
		createError = null;
		createProblem = null;
		try {
			await service.createUser({
				username: newUsername,
				display_name: newDisplayName,
				role: newRole,
				password: newPassword
			});
			pushToast('success', t('users.created'));
			createOpen = false;
			await load();
		} catch (problem) {
			createProblem = problem instanceof ApiError ? problem.problem : null;
			// A taken username is a problem of the username field (ADR-0025).
			createError =
				usernameTaken || namesField(createProblem, createFields) ? null : messageOf(problem);
		} finally {
			creating = false;
		}
	}

	function openReset(user: User): void {
		resetUser = user;
		newResetPassword = '';
		resetError = null;
		resetOpen = true;
	}

	async function resetPassword(): Promise<void> {
		if (!resetUser || newResetPassword.length < 12) return;
		resetting = true;
		resetError = null;
		try {
			await service.resetUserPassword(resetUser.id, { new_password: newResetPassword });
			pushToast('success', t('users.resetDone'));
			resetOpen = false;
			await load();
		} catch (problem) {
			resetError = messageOf(problem);
		} finally {
			resetting = false;
		}
	}

	async function toggleDisabled(user: User): Promise<void> {
		try {
			await service.updateUser(user.id, { disabled: !user.disabled });
			pushToast('success', user.disabled ? t('users.enabledToast') : t('users.disabledToast'));
			await load();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		}
	}
</script>

<SettingsLayout
	title={t('users.title')}
	description={t('users.description', { name: tenantName })}
	count={loading ? undefined : users.length}
>
	{#snippet actions()}
		<Button variant="primary" icon="plus" onclick={openCreate}>{t('users.create')}</Button>
	{/snippet}

	<Alert variant="inline" icon="users" title={t('users.sharedNote')} />

	<ListView
		label={t('users.title')}
		items={shown}
		itemKey={(user) => user.id}
		icon={(user) => iconOf(userState(user))}
		pill={(user) => pillOf(userState(user))}
		title={(user) => user.display_name}
		onOpen={openDetail}
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

	<SidePanel bind:open={detailOpen} title={detail?.display_name ?? ''}>
		<KeyValue items={detailItems} stacked />
	</SidePanel>

	<Panel title={t('users.roles')} flush>
		<ul class="roles">
			{#each roleHelp as item (item.role)}
				<li class="roles__row">
					<span class="roles__icon"><Icon name={item.icon} /></span>
					<span class="roles__text">
						<span class="roles__name">{roleKeys[item.role]}</span>
						<span class="roles__body">{t(item.body)}</span>
					</span>
				</li>
			{/each}
		</ul>
	</Panel>
</SettingsLayout>

<Dialog bind:open={createOpen} title={t('users.createTitle')} onsubmit={createUser}>
	{#if createError}
		<Alert variant="inline" tone="danger" title={createError} />
	{/if}
	<TextField
		id="user-username"
		label={t('users.username')}
		error={usernameTaken ? tErrorCode('username_taken') : fieldError(createProblem, 'username')}
		bind:value={newUsername}
		mono
		autocomplete="off"
		required
	/>
	<TextField
		id="user-display-name"
		label={t('users.displayName')}
		error={fieldError(createProblem, 'display_name')}
		bind:value={newDisplayName}
		placeholder={t('users.displayNamePlaceholder')}
		autocomplete="off"
		required
	/>
	<Select
		id="user-role"
		label={t('users.role')}
		error={fieldError(createProblem, 'role')}
		bind:value={newRole}
		options={roleOptions}
		required
	/>
	<PasswordField
		id="user-password"
		label={t('users.initialPassword')}
		help={t('setup.passwordHint')}
		error={fieldError(createProblem, 'password')}
		bind:value={newPassword}
		autocomplete="new-password"
		required
	/>
	{#snippet footer()}
		<Button onclick={() => (createOpen = false)}>{t('common.cancel')}</Button>
		<Button type="submit" variant="primary" loading={creating} disabled={!canCreate}>
			{t('users.create')}
		</Button>
	{/snippet}
</Dialog>

<Dialog bind:open={resetOpen} title={t('users.resetTitle')} onsubmit={resetPassword}>
	{#if resetError}
		<Alert variant="inline" tone="danger" title={resetError} />
	{/if}
	<p>{t('users.resetHint', { name: resetUser?.display_name ?? '' })}</p>
	<PasswordField
		id="user-reset-password"
		label={t('setup.password')}
		help={t('setup.passwordHint')}
		bind:value={newResetPassword}
		autocomplete="new-password"
		required
	/>
	{#snippet footer()}
		<Button onclick={() => (resetOpen = false)}>{t('common.cancel')}</Button>
		<Button
			type="submit"
			variant="primary"
			loading={resetting}
			disabled={newResetPassword.length < 12}
		>
			{t('users.resetPassword')}
		</Button>
	{/snippet}
</Dialog>

<Dialog bind:open={roleOpen} size="sm" title={t('users.changeRoleTitle')} onsubmit={saveRole}>
	{#if roleError}
		<Alert variant="inline" tone="danger" title={roleError} />
	{/if}
	<Select
		id="user-role-change"
		label={t('users.role')}
		options={roleOptions}
		bind:value={roleValue}
	/>
	{#snippet footer()}
		<Button onclick={() => (roleOpen = false)}>{t('common.cancel')}</Button>
		<Button type="submit" variant="primary" loading={roleSaving}>{t('common.save')}</Button>
	{/snippet}
</Dialog>

<style>
	.roles {
		margin: 0;
		padding: 0;
		list-style: none;
	}

	.roles__row {
		display: flex;
		align-items: center;
		gap: var(--rh-space-3);
		padding: var(--rh-space-3) var(--rh-space-4);
		border-top: var(--rh-border-width) solid var(--rh-color-border-muted);
	}

	.roles__row:first-child {
		border-top: 0;
	}

	.roles__icon {
		display: inline-flex;
		width: 28px;
		justify-content: center;
		color: var(--rh-color-text-muted);
	}

	.roles__text {
		display: flex;
		flex-direction: column;
	}

	.roles__name {
		font-weight: var(--rh-font-weight-semibold);
	}

	.roles__body {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}
</style>
