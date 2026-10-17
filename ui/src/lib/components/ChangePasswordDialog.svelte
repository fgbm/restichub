<script lang="ts">
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import type { ApiProblem } from '#lib/api/index.ts';
	import { Alert, Button, Dialog, NewPasswordField, PasswordField } from './ui/index.ts';
	import { emptyError, focusFirstInvalid } from '#lib/form.ts';
	import { passwordProblem } from '#lib/password-policy.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	let { open = $bindable(false) }: { open?: boolean } = $props();

	const CURRENT_ID = 'own-password-current';
	const NEW_ID = 'own-password-new';

	let currentPassword = $state('');
	let newPassword = $state('');
	let submitted = $state(false);
	let saving = $state(false);
	let problem = $state<ApiProblem | null>(null);
	let generalError = $state<string | null>(null);

	const currentError = $derived(
		emptyError(submitted, currentPassword) ??
			(problem?.field === 'current_password' && problem.reason === 'invalid_credentials'
				? t('account.password.wrongCurrent')
				: fieldError(problem, 'current_password'))
	);
	const newError = $derived(fieldError(problem, 'new_password'));

	$effect(() => {
		if (!open) return;
		currentPassword = '';
		newPassword = '';
		submitted = false;
		problem = null;
		generalError = null;
	});

	async function submit(): Promise<void> {
		submitted = true;
		problem = null;
		generalError = null;
		if (
			focusFirstInvalid([
				[CURRENT_ID, currentPassword === ''],
				[NEW_ID, passwordProblem(newPassword) !== null]
			])
		)
			return;
		saving = true;
		try {
			await service.changePassword({
				current_password: currentPassword,
				new_password: newPassword
			});
			pushToast('success', t('account.password.done'));
			open = false;
		} catch (error) {
			const found = error instanceof ApiError ? error.problem : null;
			if (found?.field === 'current_password' || found?.field === 'new_password') {
				problem = found;
				document.getElementById(found.field === 'current_password' ? CURRENT_ID : NEW_ID)?.focus();
			} else {
				generalError =
					error instanceof ApiError ? (tErrorCode(error.code) ?? error.title) : t('table.error');
			}
		} finally {
			saving = false;
		}
	}
</script>

<Dialog bind:open title={t('account.password.title')} onsubmit={submit}>
	{#if generalError}
		<Alert variant="inline" tone="danger" title={generalError} />
	{/if}
	<p>{t('account.password.hint')}</p>
	<PasswordField
		id={CURRENT_ID}
		label={t('account.password.current')}
		autocomplete="current-password"
		required
		error={currentError}
		bind:value={currentPassword}
		oninput={() => (problem = null)}
	/>
	<NewPasswordField
		id={NEW_ID}
		label={t('account.password.new')}
		{submitted}
		error={newError}
		bind:value={newPassword}
	/>
	{#snippet footer()}
		<Button onclick={() => (open = false)}>{t('common.cancel')}</Button>
		<Button type="submit" variant="primary" loading={saving}>
			{t('account.password.submit')}
		</Button>
	{/snippet}
</Dialog>
