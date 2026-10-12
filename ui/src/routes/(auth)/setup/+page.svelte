<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ApiError, NetworkError, service } from '#lib/api/index.ts';
	import { Button, FormField, InlineAlert, TextField } from '#lib/components/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	let setupToken = $state('');
	let username = $state('');
	let displayName = $state('');
	let password = $state('');
	let confirmPassword = $state('');
	let submitting = $state(false);
	let errorCode = $state<string | undefined>();
	let errorTitle = $state<string | undefined>();

	const passwordTooShort = $derived(password.length > 0 && password.length < 12);
	const passwordMismatch = $derived(confirmPassword.length > 0 && password !== confirmPassword);
	const canSubmit = $derived(
		setupToken.length > 0 &&
			username.length > 0 &&
			password.length >= 12 &&
			password === confirmPassword
	);

	async function onSubmit(event: SubmitEvent): Promise<void> {
		event.preventDefault();
		submitting = true;
		errorCode = undefined;
		errorTitle = undefined;
		try {
			await service.setup({
				setup_token: setupToken,
				username,
				password,
				display_name: displayName
			});
			pushToast('success', t('setup.done'));
			await goto(resolve('/(auth)/sign-in'));
		} catch (error) {
			if (error instanceof NetworkError) {
				errorCode = 'network';
				errorTitle = t('error.network');
			} else if (error instanceof ApiError) {
				errorCode = error.code;
				errorTitle = tErrorCode(error.code) ?? t('alert.problemTitle');
			} else {
				errorTitle = t('alert.problemTitle');
			}
		} finally {
			submitting = false;
		}
	}
</script>

<h1 class="title">{t('setup.title')}</h1>
<p class="description">{t('setup.description')}</p>

{#if errorTitle}
	<div class="alert">
		<InlineAlert tone="danger" title={errorTitle} code={errorCode} />
	</div>
{/if}

<form class="form" onsubmit={onSubmit}>
	<FormField label={t('setup.setupToken')} for="setup-token" required>
		<TextField id="setup-token" bind:value={setupToken} mono autocomplete="off" required />
	</FormField>

	<FormField label={t('setup.username')} for="setup-username" required>
		<TextField id="setup-username" bind:value={username} autocomplete="username" required />
	</FormField>

	<FormField label={t('setup.displayName')} for="setup-display-name" required>
		<TextField id="setup-display-name" bind:value={displayName} autocomplete="name" required />
	</FormField>

	<FormField
		label={t('setup.password')}
		for="setup-password"
		hint={t('setup.passwordHint')}
		error={passwordTooShort ? t('setup.passwordHint') : undefined}
		required
	>
		<TextField
			id="setup-password"
			type="password"
			bind:value={password}
			autocomplete="new-password"
			invalid={passwordTooShort}
			required
		/>
	</FormField>

	<FormField
		label={t('setup.confirmPassword')}
		for="setup-confirm"
		error={passwordMismatch ? t('setup.passwordMismatch') : undefined}
		required
	>
		<TextField
			id="setup-confirm"
			type="password"
			bind:value={confirmPassword}
			autocomplete="new-password"
			invalid={passwordMismatch}
			required
		/>
	</FormField>

	<Button type="submit" variant="primary" loading={submitting} disabled={!canSubmit}>
		{t('setup.submit')}
	</Button>
</form>

<style>
	.title {
		margin: 0;
		font-size: var(--rh-font-size-xl);
		font-weight: var(--rh-font-weight-semibold);
	}

	.description {
		margin: var(--rh-space-1) 0 0;
		color: var(--rh-color-text-muted);
	}

	.alert {
		margin-top: var(--rh-space-4);
	}

	.form {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-4);
		margin-top: var(--rh-space-5);
	}
</style>
