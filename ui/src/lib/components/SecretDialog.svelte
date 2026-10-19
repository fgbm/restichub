<script lang="ts">
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import type { ApiProblem, Secret } from '#lib/api/index.ts';
	import { Alert, Button, Dialog, TextField } from '#lib/components/ui/index.ts';
	import PasswordField from '#lib/components/ui/PasswordField.svelte';
	import { t, tErrorCode } from '#lib/i18n/index.ts';

	let {
		open = $bindable(false),
		oncreated
	}: {
		open?: boolean;
		/** Receives the stored secret. The value goes to the control plane once. */
		oncreated: (secret: Secret) => void;
	} = $props();

	let name = $state('');
	let value = $state('');
	let saving = $state(false);
	let problem = $state<ApiProblem | null>(null);
	let error = $state<string | null>(null);

	function messageOf(cause: unknown): string {
		if (cause instanceof ApiError) return tErrorCode(cause.code) ?? cause.title;
		return t('table.error');
	}

	// A dialog that opens again starts empty, so a stored value never shows twice.
	$effect(() => {
		if (!open) {
			name = '';
			value = '';
			problem = null;
			error = null;
		}
	});

	async function create(): Promise<void> {
		saving = true;
		problem = null;
		error = null;
		try {
			const created = await service.createSecret({
				name: name.trim(),
				purpose: 'hook_secret',
				value
			});
			value = '';
			open = false;
			oncreated(created);
		} catch (cause) {
			if (cause instanceof ApiError) problem = cause.problem;
			else error = messageOf(cause);
		} finally {
			saving = false;
		}
	}
</script>

<Dialog
	bind:open
	size="md"
	title={t('scripts.secret.title')}
	description={t('scripts.secret.description')}
	onsubmit={create}
>
	{#if error}
		<Alert tone="danger" title={error} />
	{/if}
	<TextField
		label={t('scripts.secret.name')}
		error={fieldError(problem, 'name')}
		bind:value={name}
		required
	/>
	<PasswordField
		label={t('scripts.secret.value')}
		error={fieldError(problem, 'value')}
		bind:value
		required
	/>
	{#snippet footer()}
		<Button onclick={() => (open = false)}>{t('common.cancel')}</Button>
		<Button type="submit" variant="primary" loading={saving}>
			{t('scripts.secret.create')}
		</Button>
	{/snippet}
</Dialog>
