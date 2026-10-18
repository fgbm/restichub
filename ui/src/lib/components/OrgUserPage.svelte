<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, assignableRoles, service } from '#lib/api/index.ts';
	import type { Role, User } from '#lib/api/index.ts';
	import OrgUserDialogs from './OrgUserDialogs.svelte';
	import { RecordPage, SummaryPanel } from './layout/index.ts';
	import { Button, DropdownMenu, Panel, Select, StatusPill } from './ui/index.ts';
	import type { KeyValueItem, MenuItem, SelectOption } from './ui/index.ts';
	import { formatTime, relativeTime } from '#lib/format.ts';
	import { t, tErrorCode, type MessageKey } from '#lib/i18n/index.ts';
	import { getSession, hasRole } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';
	import { orgChangeLock, orgResetLock } from '#lib/user-locks.ts';
	import { userState } from '#lib/vocabulary.ts';

	let { userId }: { userId: string } = $props();

	const ROLE_HELP: Partial<Record<Role, MessageKey>> = {
		admin: 'users.roles.admin',
		operator: 'users.roles.operator',
		viewer: 'users.roles.viewer'
	};

	let user = $state<User | null>(null);
	let loading = $state(true);
	let loadError = $state<string | null>(null);

	let dialogs = $state<OrgUserDialogs>();

	// The select shows this role, so a failed save returns it to the saved role.
	let roleDraft = $state<Role>('viewer');
	let roleBusy = $state(false);

	const selfId = $derived(getSession()?.user?.id ?? null);
	const name = $derived(user?.display_name ?? '');

	// An Admin changes the role, the password and the state of a member (ADR-0023, operation
	// `users_write`).
	const canWrite = $derived(hasRole('admin', 'platform_admin'));

	const roleLabels = $derived<Record<Role, string>>({
		platform_admin: t('users.role.platformAdmin'),
		admin: t('users.role.admin'),
		operator: t('users.role.operator'),
		viewer: t('users.role.viewer')
	});

	// The least privilege comes first, as in the create dialog.
	const roleOptions = $derived<SelectOption[]>(
		[...assignableRoles()].reverse().map((role) => ({ value: role, label: roleLabels[role] }))
	);

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	async function load(): Promise<void> {
		try {
			const member = await service.getUser(userId);
			user = member;
			roleDraft = member.role;
			loadError = null;
		} catch (problem) {
			loadError = messageOf(problem);
		} finally {
			loading = false;
		}
	}

	onMount(load);

	const resetLock = $derived(user ? orgResetLock(user, selfId) : null);
	const changeLock = $derived(user ? orgChangeLock(user, selfId) : null);

	const menuItems = $derived.by<MenuItem[]>(() => {
		const member = user;
		if (!member) return [];
		return [
			{
				label: member.disabled ? t('users.enableUser') : t('users.disableUser'),
				icon: 'power',
				tone: member.disabled ? 'default' : 'danger',
				disabled: Boolean(changeLock),
				hint: changeLock ?? undefined,
				onSelect: () => void dialogs?.toggleDisabled(member)
			}
		];
	});

	async function changeRole(role: Role): Promise<void> {
		if (!user || role === user.role) return;
		roleDraft = role;
		roleBusy = true;
		try {
			user = await service.updateUser(user.id, { role });
			pushToast('success', t('users.roleChanged'));
		} catch (problem) {
			roleDraft = user.role;
			pushToast('danger', messageOf(problem));
		} finally {
			roleBusy = false;
		}
	}

	const roleHelp = $derived(ROLE_HELP[roleDraft]);

	const lastSignIn = $derived(user?.last_sign_in_at ?? null);

	const accountItems = $derived.by<KeyValueItem[]>(() => {
		if (!user) return [];
		const items: KeyValueItem[] = [
			{ key: 'username', label: t('users.username'), value: user.username, mono: true },
			{ key: 'state', label: t('common.status'), value: userState(user).label }
		];
		if (user.shared) {
			items.push({ key: 'shared', label: t('users.shared'), value: t('common.yes') });
		}
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

{#snippet lastSignInValue()}
	{#if lastSignIn}
		<time datetime={lastSignIn} title={formatTime(lastSignIn)}>{relativeTime(lastSignIn)}</time>
	{/if}
{/snippet}

<RecordPage title={name} {loading} error={loadError}>
	{#snippet status()}
		{#if user?.disabled}
			<StatusPill tone="neutral">{userState(user).label}</StatusPill>
		{/if}
	{/snippet}
	{#snippet actions()}
		{#if user && canWrite}
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
		<Panel title={t('users.access')}>
			<div class="access">
				<div class="access__role">
					<Select
						id="user-role"
						label={t('users.role')}
						options={roleOptions}
						value={roleDraft}
						title={changeLock ?? undefined}
						disabled={!canWrite || roleBusy || Boolean(changeLock)}
						onchange={(event) =>
							changeRole((event.currentTarget as HTMLSelectElement).value as Role)}
					/>
				</div>
				{#if roleHelp}
					<p class="access__help">{t(roleHelp)}</p>
				{/if}
			</div>
		</Panel>
	{/if}
</RecordPage>

<OrgUserDialogs bind:this={dialogs} onchanged={load} />

<style>
	.access {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-2);
	}

	.access__role {
		max-width: 280px;
	}

	.access__help {
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}
</style>
