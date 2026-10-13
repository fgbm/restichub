<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ApiError, NetworkError, fieldError, namesField, service } from '#lib/api/index.ts';
	import type { ApiProblem } from '#lib/api/index.ts';
	import { Alert, Button, PasswordField, TextField } from '#lib/components/ui/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { isSingleTenant, loadInstance } from '#lib/stores/instance.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	let setupToken = $state('');
	let username = $state('');
	let displayName = $state('');
	let password = $state('');
	let confirmPassword = $state('');
	let submitting = $state(false);
	let errorCode = $state<string | undefined>();
	let errorTitle = $state<string | undefined>();
	let problem = $state<ApiProblem | null>(null);

	// The fields of the form. A problem of another field (for example `body`) shows the general
	// alert instead of the message next to an input.
	const formFields = ['setup_token', 'username', 'display_name', 'password'];

	// Single mode creates the tenant `default` and its first Admin. Both modes need
	// the one-time setup token, so the field stays (ADR-0024).
	const single = $derived(isSingleTenant());
	const passwordTooShort = $derived(password.length > 0 && password.length < 12);
	const passwordMismatch = $derived(confirmPassword.length > 0 && password !== confirmPassword);
	const canSubmit = $derived(
		setupToken.length > 0 &&
			username.length > 0 &&
			password.length >= 12 &&
			password === confirmPassword
	);

	onMount(() => {
		void loadInstance();
	});

	async function onSubmit(event: SubmitEvent): Promise<void> {
		event.preventDefault();
		submitting = true;
		errorCode = undefined;
		errorTitle = undefined;
		problem = null;
		try {
			await service.setup({
				setup_token: setupToken,
				username,
				password,
				display_name: displayName
			});
			pushToast('success', t(single ? 'setup.doneSingle' : 'setup.done'));
			await goto(resolve('/(auth)/sign-in'));
		} catch (error) {
			if (error instanceof NetworkError) {
				errorCode = 'network';
				errorTitle = t('error.network');
			} else if (error instanceof ApiError) {
				problem = error.problem;
				if (!namesField(problem, formFields)) {
					errorCode = error.code;
					errorTitle = tErrorCode(error.code) ?? t('alert.problemTitle');
				}
			} else {
				errorTitle = t('alert.problemTitle');
			}
		} finally {
			submitting = false;
		}
	}
</script>

<svelte:head>
	<title>{t('setup.title')} · {t('app.name')}</title>
</svelte:head>

<h1 class="title">{t('setup.title')}</h1>
<p class="description">{t(single ? 'setup.descriptionSingle' : 'setup.description')}</p>

{#if errorTitle}
	<div class="alert">
		<Alert tone="danger" title={errorTitle} code={errorCode} />
	</div>
{/if}

<form class="form" onsubmit={onSubmit}>
	<TextField
		id="setup-token"
		label={t('setup.setupToken')}
		help={t('setup.tokenHint')}
		error={fieldError(problem, 'setup_token')}
		bind:value={setupToken}
		mono
		autocomplete="off"
		required
	/>
	<TextField
		id="setup-username"
		label={t('setup.username')}
		error={fieldError(problem, 'username')}
		bind:value={username}
		autocomplete="username"
		required
	/>
	<TextField
		id="setup-display-name"
		label={t('setup.displayName')}
		error={fieldError(problem, 'display_name')}
		bind:value={displayName}
		autocomplete="name"
		required
	/>
	<PasswordField
		id="setup-password"
		label={t('setup.password')}
		help={t('setup.passwordHint')}
		error={passwordTooShort ? t('setup.passwordHint') : fieldError(problem, 'password')}
		bind:value={password}
		autocomplete="new-password"
		required
	/>
	<PasswordField
		id="setup-confirm"
		label={t('setup.confirmPassword')}
		error={passwordMismatch ? t('setup.passwordMismatch') : undefined}
		bind:value={confirmPassword}
		autocomplete="new-password"
		required
	/>
	<Button type="submit" variant="primary" loading={submitting} disabled={!canSubmit}>
		{t(single ? 'setup.submitSingle' : 'setup.submit')}
	</Button>
</form>

<p class="back">
	<a href={resolve('/(auth)/sign-in')}>{t('setup.backToSignIn')}</a>
</p>

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

	.back {
		margin: var(--rh-space-4) 0 0;
		font-size: var(--rh-font-size-sm);
	}
</style>
