<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import type { PathnameWithSearchOrHash } from '$app/types';
	import { ApiError, NetworkError, service } from '#lib/api/index.ts';
	import { Button, FormField, InlineAlert, TextField } from '#lib/components/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { setSession } from '#lib/stores/session.svelte.ts';

	let tenant = $state('');
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

	// An installation without a user needs the setup first. The sign-in form stays
	// in place when the check fails.
	onMount(async () => {
		try {
			const status = await service.setupStatus();
			if (status.setup_required) await goto(resolve('/(auth)/setup'));
		} catch {
			// The form stays.
		}
	});

	async function onSubmit(event: SubmitEvent): Promise<void> {
		event.preventDefault();
		submitting = true;
		errorCode = undefined;
		errorTitle = undefined;
		try {
			const me = await service.signIn({ tenant, username, password });
			setSession(me);
			await goto(resolve(next.replace(/^\//, '') as PathnameWithSearchOrHash));
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

<h1 class="title">{t('signIn.title')}</h1>
<p class="description">{t('signIn.description')}</p>

{#if errorTitle}
	<div class="alert">
		<InlineAlert tone="danger" title={errorTitle} code={errorCode} />
	</div>
{/if}

<form class="form" onsubmit={onSubmit}>
	<FormField label={t('signIn.tenant')} for="signin-tenant" required>
		<TextField
			id="signin-tenant"
			bind:value={tenant}
			autocomplete="organization"
			placeholder={t('signIn.tenantPlaceholder')}
			required
		/>
	</FormField>

	<FormField label={t('signIn.username')} for="signin-username" required>
		<TextField id="signin-username" bind:value={username} autocomplete="username" required />
	</FormField>

	<FormField label={t('signIn.password')} for="signin-password" required>
		<TextField
			id="signin-password"
			type="password"
			bind:value={password}
			autocomplete="current-password"
			required
		/>
	</FormField>

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
