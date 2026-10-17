<script lang="ts">
	import { t } from '#lib/i18n/index.ts';
	import type { BlockedView } from '#lib/delete-check.ts';
	import Alert from './Alert.svelte';
	import Button from './Button.svelte';
	import Dialog from './Dialog.svelte';
	import type { IconName } from './icons';

	let {
		open = $bindable(false),
		title,
		message,
		confirmLabel,
		cancelLabel = t('common.cancel'),
		tone = 'primary',
		icon,
		loading = false,
		blocked = null,
		checking = false,
		onConfirm
	}: {
		open?: boolean;
		title: string;
		message: string;
		confirmLabel: string;
		cancelLabel?: string;
		tone?: 'primary' | 'danger';
		/** Title icon; a destructive confirmation uses `octagon`. */
		icon?: IconName;
		loading?: boolean;
		/** The reason the action is refused. The confirm button stays disabled. */
		blocked?: BlockedView | null;
		/** The dialog waits for the usage check, so the button stays disabled. */
		checking?: boolean;
		onConfirm?: () => void;
	} = $props();
</script>

<Dialog bind:open {title} {icon} description={message} size="sm" closable={false}>
	{#if checking}
		<p class="hint">{t('deleteBlocked.checking')}</p>
	{:else if blocked}
		<Alert tone="danger" title={blocked.reason} />
		{#if blocked.links.length > 0}
			<ul class="blockers">
				{#each blocked.links as link (link.href + link.label)}
					<li>
						<span class="blockers__kind">{link.kind}</span>
						<!-- The caller resolves the address with `resolve()`. -->
						<!-- eslint-disable-next-line svelte/no-navigation-without-resolve -->
						<a href={link.href}>{link.label}</a>
					</li>
				{/each}
				{#if blocked.more > 0}
					<li class="blockers__more">{t('deleteBlocked.more', { n: blocked.more })}</li>
				{/if}
			</ul>
		{/if}
	{/if}
	{#snippet footer()}
		<Button variant="secondary" onclick={() => (open = false)}>{cancelLabel}</Button>
		<span title={blocked ? t('deleteBlocked.disabled') : undefined}>
			<Button
				variant={tone}
				{loading}
				disabled={checking || blocked !== null}
				onclick={() => onConfirm?.()}
			>
				{confirmLabel}
			</Button>
		</span>
	{/snippet}
</Dialog>

<style>
	.hint {
		margin: 0;
		color: var(--rh-color-text-subtle);
	}

	.blockers {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-1);
		margin: var(--rh-space-2) 0 0;
		padding: 0;
		list-style: none;
	}

	.blockers__kind {
		color: var(--rh-color-text-subtle);
		margin-right: var(--rh-space-1);
	}

	.blockers__more {
		color: var(--rh-color-text-subtle);
	}
</style>
