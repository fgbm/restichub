<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, assignableRoles, fieldError, namesField, service } from '#lib/api/index.ts';
	import type { ApiProblem, Role, User } from '#lib/api/index.ts';
	import { SettingsLayout } from '#lib/components/index.ts';
	import {
		Alert,
		Badge,
		Button,
		Dialog,
		DropdownMenu,
		Icon,
		Panel,
		PasswordField,
		Select,
		StatusPill,
		Table,
		TextField
	} from '#lib/components/ui/index.ts';
	import type { IconName, SelectOption, TableColumn } from '#lib/components/ui/index.ts';
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
	const roles = $derived([...assignableRoles(getSession()?.role)].reverse());

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

	function initials(user: User): string {
		return (user.display_name || user.username)
			.split(/\s+/)
			.filter(Boolean)
			.slice(0, 2)
			.map((part) => part[0]?.toUpperCase() ?? '')
			.join('');
	}

	const columns = $derived<TableColumn<User>[]>([
		{
			key: 'display_name',
			header: t('users.user'),
			value: (row) => `${row.display_name} ${row.username}`,
			render: userCell
		},
		{
			key: 'role',
			header: t('users.role'),
			cell: (row) => roleKeys[row.role],
			badge: () => 'neutral'
		},
		{
			key: 'disabled',
			header: t('common.status'),
			value: (row) => (row.disabled ? t('common.disabled') : t('common.active')),
			render: stateCell
		}
	]);

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

{#snippet userCell(user: User)}
	<span class="user">
		<span class="user__avatar" aria-hidden="true">{initials(user)}</span>
		<span class="user__text">
			<span class="user__line">
				<span class="user__name">{user.display_name}</span>
				{#if user.shared}
					<Badge icon="users">{t('users.shared')}</Badge>
				{/if}
			</span>
			<span class="user__login">{user.username}</span>
		</span>
	</span>
{/snippet}

{#snippet stateCell(user: User)}
	<StatusPill tone={user.disabled ? 'neutral' : 'success'}>
		{user.disabled ? t('common.disabled') : t('common.active')}
	</StatusPill>
{/snippet}

<svelte:head>
	<title>{t('users.title')} · {t('app.name')}</title>
</svelte:head>

<SettingsLayout
	title={t('users.title')}
	description={t('users.description', { name: tenantName })}
	count={loading ? undefined : users.length}
>
	{#snippet actions()}
		<Button variant="primary" icon="plus" onclick={openCreate}>{t('users.create')}</Button>
	{/snippet}

	<Alert variant="inline" icon="users" title={t('users.sharedNote')} />

	<Table
		{columns}
		rows={users}
		{loading}
		{error}
		rowKey={(user) => user.id}
		filterPlaceholder={t('users.searchPlaceholder')}
	>
		{#snippet rowActions(user)}
			<span class="row-actions">
				<Button
					size="sm"
					icon="key"
					disabled={user.shared}
					title={user.shared ? t('users.sharedResetHint') : t('users.resetPassword')}
					aria-label={t('users.resetFor', { name: user.display_name })}
					onclick={() => openReset(user)}
				/>
				<DropdownMenu
					icon="more"
					align="end"
					label={t('users.moreActions', { name: user.display_name })}
					items={[
						{
							label: user.disabled ? t('users.enableUser') : t('users.disableUser'),
							icon: 'octagon',
							tone: 'danger',
							onSelect: () => toggleDisabled(user)
						}
					]}
				/>
			</span>
		{/snippet}
	</Table>

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

<style>
	.user {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.user__avatar {
		display: inline-flex;
		flex: none;
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

	.user__text {
		display: flex;
		flex-direction: column;
	}

	.user__line {
		display: inline-flex;
		align-items: center;
		gap: var(--rh-space-3);
	}

	.user__name {
		font-weight: var(--rh-font-weight-semibold);
	}

	.user__login {
		color: var(--rh-color-text-muted);
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-sm);
	}

	.row-actions {
		display: inline-flex;
		gap: var(--rh-space-1);
	}

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
