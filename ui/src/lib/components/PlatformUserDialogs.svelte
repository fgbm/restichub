<script lang="ts">
	import { ApiError, service } from '#lib/api/index.ts';
	import type { PlatformUser } from '#lib/api/index.ts';
	import { Alert, Button, ConfirmDialog, Dialog, NewPasswordField } from './ui/index.ts';
	import { focusFirstInvalid } from '#lib/form.ts';
	import { passwordProblem } from '#lib/password-policy.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	let { onchanged }: { onchanged: () => void | Promise<void> } = $props();

	let target = $state<PlatformUser | null>(null);
	const name = $derived(target?.display_name ?? '');

	let resetOpen = $state(false);
	let resetPassword = $state('');
	let resetting = $state(false);
	let resetSubmitted = $state(false);
	let resetError = $state<string | null>(null);

	// A grant of the right asks for a new password: the old password may be known to a tenant Admin
	// (ADR-0029).
	let grantOpen = $state(false);
	let grantPassword = $state('');
	let granting = $state(false);
	let grantSubmitted = $state(false);
	let grantError = $state<string | null>(null);

	let revokeOpen = $state(false);
	let revoking = $state(false);

	let disableOpen = $state(false);
	let disabling = $state(false);

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	export function openReset(user: PlatformUser): void {
		target = user;
		resetPassword = '';
		resetError = null;
		resetSubmitted = false;
		resetOpen = true;
	}

	export function openGrant(user: PlatformUser): void {
		target = user;
		grantPassword = '';
		grantError = null;
		grantSubmitted = false;
		grantOpen = true;
	}

	export function openRevoke(user: PlatformUser): void {
		target = user;
		revokeOpen = true;
	}

	/** Disabling asks first; enabling is safe and applies at once. */
	export async function toggleDisabled(user: PlatformUser): Promise<void> {
		if (!user.disabled) {
			target = user;
			disableOpen = true;
			return;
		}
		try {
			await service.updatePlatformUser(user.id, { disabled: false });
			pushToast('success', t('users.enabledToast'));
			await onchanged();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		}
	}

	async function submitReset(): Promise<void> {
		if (!target) return;
		resetSubmitted = true;
		if (
			focusFirstInvalid([['platform-user-reset-password', passwordProblem(resetPassword) !== null]])
		)
			return;
		resetting = true;
		resetError = null;
		try {
			await service.resetPlatformUserPassword(target.id, { new_password: resetPassword });
			pushToast('success', t('users.resetDone'));
			resetOpen = false;
		} catch (problem) {
			resetError = messageOf(problem);
		} finally {
			resetPassword = '';
			resetting = false;
		}
	}

	async function submitGrant(): Promise<void> {
		if (!target) return;
		grantSubmitted = true;
		if (
			focusFirstInvalid([['platform-user-grant-password', passwordProblem(grantPassword) !== null]])
		)
			return;
		granting = true;
		grantError = null;
		try {
			await service.updatePlatformUser(target.id, {
				platform_admin: true,
				password: grantPassword
			});
			pushToast('success', t('platformUsers.rightGranted'));
			grantOpen = false;
			await onchanged();
		} catch (problem) {
			grantError = messageOf(problem);
		} finally {
			grantPassword = '';
			granting = false;
		}
	}

	async function confirmRevoke(): Promise<void> {
		if (!target) return;
		revoking = true;
		try {
			await service.updatePlatformUser(target.id, { platform_admin: false });
			pushToast('success', t('platformUsers.rightRemoved'));
			revokeOpen = false;
			await onchanged();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			revoking = false;
		}
	}

	async function confirmDisable(): Promise<void> {
		if (!target) return;
		disabling = true;
		try {
			await service.updatePlatformUser(target.id, { disabled: true });
			pushToast('success', t('users.disabledToast'));
			disableOpen = false;
			await onchanged();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			disabling = false;
		}
	}
</script>

<Dialog bind:open={resetOpen} title={t('users.resetTitle')} onsubmit={submitReset}>
	{#if resetError}
		<Alert variant="inline" tone="danger" title={resetError} />
	{/if}
	<p>{t('users.resetHint', { name })}</p>
	<NewPasswordField
		id="platform-user-reset-password"
		label={t('setup.password')}
		submitted={resetSubmitted}
		bind:value={resetPassword}
	/>
	{#snippet footer()}
		<Button onclick={() => (resetOpen = false)}>{t('common.cancel')}</Button>
		<Button type="submit" variant="primary" loading={resetting}>
			{t('users.resetPassword')}
		</Button>
	{/snippet}
</Dialog>

<Dialog
	bind:open={grantOpen}
	icon="shield"
	title={t('platformUsers.grantTitle')}
	onsubmit={submitGrant}
>
	{#if grantError}
		<Alert variant="inline" tone="danger" title={grantError} />
	{/if}
	<p>{t('platformUsers.grantHint', { name })}</p>
	<NewPasswordField
		id="platform-user-grant-password"
		label={t('setup.password')}
		submitted={grantSubmitted}
		bind:value={grantPassword}
	/>
	{#snippet footer()}
		<Button onclick={() => (grantOpen = false)}>{t('common.cancel')}</Button>
		<Button type="submit" variant="primary" loading={granting}>
			{t('platformUsers.grantAdmin')}
		</Button>
	{/snippet}
</Dialog>

<ConfirmDialog
	bind:open={revokeOpen}
	icon="octagon"
	title={t('platformUsers.revokeTitle')}
	message={t('platformUsers.revokeBody', { name })}
	confirmLabel={t('platformUsers.revokeConfirm')}
	tone="danger"
	loading={revoking}
	onConfirm={confirmRevoke}
/>

<ConfirmDialog
	bind:open={disableOpen}
	icon="octagon"
	title={t('users.disableTitle')}
	message={t('platformUsers.disableBody', { name })}
	confirmLabel={t('users.disableUser')}
	tone="danger"
	loading={disabling}
	onConfirm={confirmDisable}
/>
