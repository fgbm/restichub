<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError, fieldError, service } from '#lib/api/index.ts';
	import type { AlertKind, AlertRule, ApiProblem } from '#lib/api/index.ts';
	import { SettingsLayout } from '#lib/components/index.ts';
	import { Alert, Button, Panel, Skeleton, Switch, TextField } from '#lib/components/ui/index.ts';
	import { t, tErrorCode } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { getSession } from '#lib/stores/session.svelte.ts';
	import { pushToast } from '#lib/stores/toasts.svelte.ts';

	interface Row {
		kind: AlertKind;
		enabled: boolean;
		/** The text of the input, in minutes. Empty for a kind without a threshold. */
		minutes: string;
		hasThreshold: boolean;
	}

	// An Admin changes the rules of the organization (ADR-0036, operation `alerts_write`).
	const canWrite = $derived(['admin', 'platform_admin'].includes(getSession()?.role ?? ''));

	let rows = $state<Row[]>([]);
	let loading = $state(true);
	let saving = $state(false);
	let error = $state<string | null>(null);
	let problem = $state<ApiProblem | null>(null);

	function messageOf(cause: unknown): string {
		if (cause instanceof ApiError) return tErrorCode(cause.code) ?? cause.title;
		return t('table.error');
	}

	// The mock throws a plain error that holds the problem, the way the plans screen reads it.
	function problemOf(cause: unknown): ApiProblem | null {
		if (cause instanceof ApiError) return cause.problem;
		if (cause && typeof cause === 'object' && 'problem' in cause) {
			return (cause as { problem: ApiProblem }).problem;
		}
		return null;
	}

	function toRows(rules: AlertRule[]): Row[] {
		return rules.map((rule) => ({
			kind: rule.kind,
			enabled: rule.enabled,
			minutes: rule.threshold_minutes == null ? '' : String(rule.threshold_minutes),
			hasThreshold: rule.kind === 'agent_offline' || rule.kind === 'stale_lock'
		}));
	}

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			rows = toRows(await service.listAlertRules());
		} catch (cause) {
			error = messageOf(cause);
		} finally {
			loading = false;
		}
	}

	onMount(load);

	async function save(): Promise<void> {
		saving = true;
		error = null;
		problem = null;
		try {
			const items = rows.map((row) => ({
				kind: row.kind,
				enabled: row.enabled,
				threshold_minutes: row.hasThreshold ? Math.round(Number(row.minutes) || 0) : null
			}));
			rows = toRows(await service.updateAlertRules({ items }));
			pushToast('success', t('rules.saved'));
		} catch (cause) {
			problem = problemOf(cause);
			error = problem?.field === 'threshold_minutes' ? null : messageOf(cause);
		} finally {
			saving = false;
		}
	}
</script>

<SettingsLayout title={t('rules.title')} description={t('rules.description')}>
	{#snippet actions()}
		{#if canWrite}
			<Button variant="primary" loading={saving} disabled={loading} onclick={save}>
				{t('rules.save')}
			</Button>
		{/if}
	{/snippet}

	{#if error}
		<Alert tone="danger" title={error} />
	{/if}

	<Panel flush>
		{#if loading}
			<div class="rules__loading"><Skeleton variant="row" /></div>
		{:else}
			<ul class="rules">
				{#each rows as row (row.kind)}
					<li class="rule">
						<div class="rule__main">
							<p class="rule__title">{t(`rules.kind.${row.kind}.title` as MessageKey)}</p>
							<p class="rule__body">{t(`rules.kind.${row.kind}.body` as MessageKey)}</p>
						</div>
						{#if row.hasThreshold}
							<div class="rule__threshold">
								<TextField
									id={`rule-${row.kind}-minutes`}
									label={t('rules.threshold')}
									inputmode="numeric"
									disabled={!canWrite || !row.enabled}
									error={fieldError(problem, 'threshold_minutes')}
									bind:value={row.minutes}
								/>
							</div>
						{/if}
						<Switch
							id={`rule-${row.kind}`}
							label={t('rules.send')}
							disabled={!canWrite}
							bind:checked={row.enabled}
						/>
					</li>
				{/each}
			</ul>
		{/if}
	</Panel>
</SettingsLayout>

<style>
	.rules {
		margin: 0;
		padding: 0;
		list-style: none;
	}

	.rules__loading {
		padding: var(--rh-space-4);
	}

	.rule {
		display: flex;
		align-items: center;
		gap: var(--rh-space-4);
		padding: var(--rh-space-4);
		border-top: var(--rh-border-width) solid var(--rh-color-border-muted);
	}

	.rule:first-child {
		border-top: 0;
	}

	.rule__main {
		flex: 1 1 auto;
		min-width: 0;
	}

	.rule__title {
		margin: 0;
		font-weight: var(--rh-font-weight-semibold);
	}

	.rule__body {
		margin: 2px 0 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.rule__threshold {
		flex: none;
		width: 9rem;
	}
</style>
