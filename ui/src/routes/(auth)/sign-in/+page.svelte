<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { ApiError, NetworkError, service } from '#lib/api/index.ts';
	import { Alert, Button, PasswordField, TextField } from '#lib/components/ui/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { getSetupRequired, loadInstance } from '#lib/stores/instance.svelte.ts';
	import { getSession, isLoaded, loadSession, setSession } from '#lib/stores/session.svelte.ts';

	let username = $state('');
	let password = $state('');
	let submitting = $state(false);
	let errorCode = $state<string | undefined>();
	let errorTitle = $state<string | undefined>();

	/**
	 * Read the `next` parameter. Only a path inside the application is accepted,
	 * so a crafted value cannot redirect to another site.
	 */
	function safeNext(value: string | null): string {
		if (value && /^\/(?![/\\])/.test(value)) return value;
		return '/';
	}

	const next = $derived(safeNext(page.url.searchParams.get('next')));

	// An installation without a user needs the setup first. A signed-in person has no
	// reason to see the sign-in form, so the screen moves them to the dashboard. The
	// probe is safe here: the API client never redirects on a 401 while the sign-in
	// path is open (NO_REDIRECT_PATHS and shouldRedirectToSignIn in client.ts). The
	// form stays when the session is unknown, or a lost session would loop.
	onMount(async () => {
		await loadInstance();
		if (getSetupRequired() === true) {
			await goto(resolve('/(auth)/setup'));
			return;
		}
		if (!isLoaded()) await loadSession();
		if (getSession()) {
			// The `next` value is a validated in-app path and the base path is empty.
			// eslint-disable-next-line svelte/no-navigation-without-resolve
			await goto(next);
		}
	});

	async function onSubmit(event: SubmitEvent): Promise<void> {
		event.preventDefault();
		submitting = true;
		errorCode = undefined;
		errorTitle = undefined;
		try {
			// The account is installation-wide; the sign-in asks for no tenant (ADR-0025).
			const me = await service.signIn({ username, password });
			setSession(me);
			// The `next` value is a validated in-app path and the base path is empty.
			// eslint-disable-next-line svelte/no-navigation-without-resolve
			await goto(next);
		} catch (error) {
			if (error instanceof NetworkError) {
				errorCode = 'network';
				errorTitle = t('error.network');
			} else if (error instanceof ApiError) {
				errorCode = error.code;
				errorTitle = tErrorCode(error.code) ?? t('signIn.invalid');
			} else {
				errorTitle = t('alert.problemTitle');
			}
		} finally {
			submitting = false;
		}
	}
</script>

<svelte:head>
	<title>{t('signIn.title')} · {t('app.name')}</title>
</svelte:head>

<h1 class="title">{t('signIn.title')}</h1>
<p class="description">{t('signIn.description')}</p>

{#if errorTitle}
	<div class="alert">
		<Alert tone="danger" title={errorTitle} code={errorCode} />
	</div>
{/if}

<form class="form" onsubmit={onSubmit}>
	<TextField
		id="signin-username"
		label={t('signIn.username')}
		bind:value={username}
		autocomplete="username"
		required
	/>
	<PasswordField
		id="signin-password"
		label={t('signIn.password')}
		bind:value={password}
		autocomplete="current-password"
		required
	/>
	<Button type="submit" variant="primary" loading={submitting}>
		{submitting ? t('signIn.submitting') : t('signIn.submit')}
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
