<script lang="ts">
	import { ApiError, service } from '#lib/api/index.ts';
	import type { User } from '#lib/api/index.ts';
	import { Alert, Button, ConfirmDialog, Dialog, NewPasswordField } from './ui/index.ts';
	import { focusFirstInvalid } from '#lib/form.ts';
	import { passwordProblem } from '#lib/password-policy.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	let { onchanged }: { onchanged: () => void | Promise<void> } = $props();

	let target = $state<User | null>(null);
	const name = $derived(target?.display_name ?? '');
	const tenantName = $derived(getSession()?.tenant?.name ?? '');

	let resetOpen = $state(false);
	let resetPassword = $state('');
	let resetting = $state(false);
	let resetSubmitted = $state(false);
	let resetError = $state<string | null>(null);

	let disableOpen = $state(false);
	let disabling = $state(false);

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	export function openReset(user: User): void {
		target = user;
		resetPassword = '';
		resetError = null;
		resetSubmitted = false;
		resetOpen = true;
	}

	/** Disabling asks first; enabling is safe and applies at once. */
	export async function toggleDisabled(user: User): Promise<void> {
		if (!user.disabled) {
			target = user;
			disableOpen = true;
			return;
		}
		try {
			await service.updateUser(user.id, { disabled: false });
			pushToast('success', t('users.enabledToast'));
			await onchanged();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		}
	}

	async function submitReset(): Promise<void> {
		if (!target) return;
		resetSubmitted = true;
		if (focusFirstInvalid([['user-reset-password', passwordProblem(resetPassword) !== null]]))
			return;
		resetting = true;
		resetError = null;
		try {
			await service.resetUserPassword(target.id, { new_password: resetPassword });
			pushToast('success', t('users.resetDone'));
			resetOpen = false;
		} catch (problem) {
			resetError = messageOf(problem);
		} finally {
			resetPassword = '';
			resetting = false;
		}
	}

	async function confirmDisable(): Promise<void> {
		if (!target) return;
		disabling = true;
		try {
			await service.updateUser(target.id, { disabled: true });
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
		id="user-reset-password"
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

<ConfirmDialog
	bind:open={disableOpen}
	icon="octagon"
	title={t('users.disableTitle')}
	message={t('users.disableBody', { name, org: tenantName })}
	confirmLabel={t('users.disableUser')}
	tone="danger"
	loading={disabling}
	onConfirm={confirmDisable}
/>
