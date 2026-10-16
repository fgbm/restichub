<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import type { Agent, ApiProblem, EnrollmentTokenCreated } from '#lib/api/index.ts';
	import { Alert, Button, ConfirmDialog, CopyField, Dialog, TextField } from './ui/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import { relativeTime } from '#lib/format.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	// The host name the control plane accepts (validate_display_name).
	const HOSTNAME_MAX_LENGTH = 128;

	let { onchanged }: { onchanged: () => void | Promise<void> } = $props();

	let renameTarget = $state<Agent | null>(null);
	let renameOpen = $state(false);
	let renaming = $state(false);
	let renameName = $state('');
	let renameError = $state<string | null>(null);
	let renameProblem = $state<ApiProblem | null>(null);

	let revokeTarget = $state<Agent | null>(null);
	let revokeOpen = $state(false);
	let revoking = $state(false);

	let reregisterTarget = $state<Agent | null>(null);
	let reregisterOpen = $state(false);
	let reregistering = $state(false);
	let reregisterToken = $state<EnrollmentTokenCreated | null>(null);
	let reregisterTokenOpen = $state(false);

	let replaceTarget = $state<Agent | null>(null);
	let replaceOpen = $state(false);
	let replacing = $state(false);

	// The registration command carries this installation URL (ADR-0032).
	let origin = $state('');
	onMount(() => {
		origin = window.location.origin;
	});
	const reregisterCommand = $derived(
		t('hosts.reregisterCommand', { url: origin, token: reregisterToken?.token ?? '<token>' })
	);

	const renameLength = $derived(Array.from(renameName.trim()).length);
	const renameValid = $derived(renameLength >= 1 && renameLength <= HOSTNAME_MAX_LENGTH);
	const renameFieldError = $derived(
		renameLength > HOSTNAME_MAX_LENGTH
			? t('hosts.renameTooLong')
			: fieldError(renameProblem, 'hostname')
	);

	function messageOf(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		return t('table.error');
	}

	export function openRename(agent: Agent): void {
		renameTarget = agent;
		renameName = agent.hostname;
		renameError = null;
		renameProblem = null;
		renameOpen = true;
	}

	export function openReregister(agent: Agent): void {
		reregisterTarget = agent;
		reregisterOpen = true;
	}

	export function openReplace(agent: Agent): void {
		replaceTarget = agent;
		replaceOpen = true;
	}

	export function openRevoke(agent: Agent): void {
		revokeTarget = agent;
		revokeOpen = true;
	}

	async function renameHost(): Promise<void> {
		if (!renameTarget || !renameValid) return;
		renaming = true;
		renameError = null;
		renameProblem = null;
		try {
			await service.renameAgent(renameTarget.id, renameName.trim());
			pushToast('success', t('hosts.renamed'));
			renameOpen = false;
			await onchanged();
		} catch (problem) {
			renameProblem = problem instanceof ApiError ? problem.problem : null;
			renameError =
				renameLength > HOSTNAME_MAX_LENGTH || fieldError(renameProblem, 'hostname')
					? null
					: messageOf(problem);
		} finally {
			renaming = false;
		}
	}

	async function confirmReregister(): Promise<void> {
		if (!reregisterTarget) return;
		reregistering = true;
		try {
			reregisterToken = await service.reenrollAgent(reregisterTarget.id);
			reregisterOpen = false;
			reregisterTokenOpen = true;
			await onchanged();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			reregistering = false;
		}
	}

	// The token value is shown once. Clear it whenever the dialog closes.
	$effect(() => {
		if (!reregisterTokenOpen) reregisterToken = null;
	});

	async function confirmReplace(): Promise<void> {
		const old = replaceTarget?.possible_duplicate_of;
		if (!replaceTarget || !old) return;
		replacing = true;
		try {
			const moved = await service.replaceAgent(replaceTarget.id, old.id);
			pushToast('success', t('hosts.replaced', { plans: moved.plans_moved }));
			replaceOpen = false;
			replaceTarget = null;
			await onchanged();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			replacing = false;
		}
	}

	async function confirmRevoke(): Promise<void> {
		if (!revokeTarget) return;
		revoking = true;
		try {
			await service.revokeAgent(revokeTarget.id);
			pushToast('success', t('hosts.revoked'));
			revokeOpen = false;
			revokeTarget = null;
			await onchanged();
		} catch (problem) {
			pushToast('danger', messageOf(problem));
		} finally {
			revoking = false;
		}
	}
</script>

<ConfirmDialog
	bind:open={reregisterOpen}
	title={t('hosts.reregisterTitle')}
	message={t('hosts.reregisterBody', { name: reregisterTarget?.hostname ?? '' })}
	confirmLabel={t('hosts.reregisterCreate')}
	cancelLabel={t('common.cancel')}
	loading={reregistering}
	onConfirm={confirmReregister}
/>

<Dialog bind:open={reregisterTokenOpen} size="lg" title={t('hosts.reregisterTokenTitle')}>
	{#if reregisterToken}
		<Alert tone="attention" icon="key" title={t('tokens.valueOnceBody')}>
			{t('hosts.reregisterNote', { name: reregisterTarget?.hostname ?? '' })}
		</Alert>
		<CopyField value={reregisterToken.token} label={t('hosts.tokenValue')} secret />
		<CopyField value={reregisterCommand} label={t('hosts.installCommand')} />
	{/if}
	{#snippet footer()}
		<Button variant="primary" onclick={() => (reregisterTokenOpen = false)}>
			{t('common.done')}
		</Button>
	{/snippet}
</Dialog>

<ConfirmDialog
	bind:open={replaceOpen}
	title={t('hosts.replaceTitle')}
	message={t('hosts.replaceBody', {
		old: replaceTarget?.possible_duplicate_of?.hostname ?? '',
		oldSeen: replaceTarget?.possible_duplicate_of?.last_seen_at
			? relativeTime(replaceTarget.possible_duplicate_of.last_seen_at)
			: t('common.never'),
		newSeen: replaceTarget?.last_seen_at
			? relativeTime(replaceTarget.last_seen_at)
			: t('common.never')
	})}
	confirmLabel={t('hosts.replace')}
	cancelLabel={t('common.cancel')}
	tone="danger"
	loading={replacing}
	onConfirm={confirmReplace}
/>

<Dialog bind:open={renameOpen} size="sm" title={t('hosts.renameTitle')} onsubmit={renameHost}>
	{#if renameError}
		<Alert tone="danger" title={renameError} />
	{/if}
	<TextField
		id="host-rename"
		label={t('hosts.hostname')}
		help={t('hosts.renameHint')}
		error={renameFieldError}
		bind:value={renameName}
		maxlength={HOSTNAME_MAX_LENGTH}
		required
	/>
	{#snippet footer()}
		<Button onclick={() => (renameOpen = false)}>{t('common.cancel')}</Button>
		<Button type="submit" variant="primary" loading={renaming} disabled={!renameValid}>
			{t('hosts.rename')}
		</Button>
	{/snippet}
</Dialog>

<Dialog bind:open={revokeOpen} size="sm" icon="octagon" title={t('hosts.revokeTitle')}>
	<p>
		{t('hosts.revokeBody', { name: revokeTarget?.hostname ?? '' })}
	</p>
	{#snippet footer()}
		<Button onclick={() => (revokeOpen = false)}>{t('common.cancel')}</Button>
		<Button variant="danger" loading={revoking} onclick={confirmRevoke}>
			{t('hosts.revoke')}
		</Button>
	{/snippet}
</Dialog>
