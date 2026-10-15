<script lang="ts">
	import { resolve } from '$app/paths';
	import type { ScriptEnvKind, Secret } from '#lib/api/index.ts';
	import { Badge, Button, Select, TextField } from '#lib/components/ui/index.ts';
	import type { SelectOption } from '#lib/components/ui/index.ts';
	import { t } from '#lib/i18n/index.ts';
	import type { MessageKey } from '#lib/i18n/index.ts';
	import { AFTER_PHASES, INTERPRETERS, MAX_BODY_BYTES, byteLength } from '#lib/scripts.ts';
	import type { Group, ScriptDraft } from '#lib/scripts.ts';
	import CodeEditor from './CodeEditor.svelte';

	let {
		script = $bindable(),
		number,
		group,
		secrets,
		readonly = false,
		canUp,
		canDown,
		error,
		onUp,
		onDown,
		onRemove,
		onNewSecret
	}: {
		script: ScriptDraft;
		/** The position of the script in its list, counted from 1. */
		number: number;
		group: Group;
		secrets: Secret[];
		readonly?: boolean;
		canUp: boolean;
		canDown: boolean;
		/** The message of a field of this script, such as `body` or `env[0].value`. */
		error: (field: string) => string | undefined;
		onUp: () => void;
		onDown: () => void;
		onRemove: () => void;
		/** Opens the dialog that creates a secret for the parameter at this position. */
		onNewSecret: (parameter: number) => void;
	} = $props();

	const interpreterOptions = $derived<SelectOption[]>(
		INTERPRETERS.map((value) => ({
			value,
			label: t(`scripts.interpreter.${value}` as MessageKey)
		}))
	);
	const phaseOptions = $derived<SelectOption[]>(
		AFTER_PHASES.map((value) => ({
			value,
			label: t(`profiles.hooks.phase.${value}` as MessageKey)
		}))
	);
	const policyOptions = $derived<SelectOption[]>([
		{ value: 'abort_backup', label: t('scripts.policy.abort_backup') },
		{ value: 'continue', label: t('scripts.policy.continue') }
	]);
	const kindOptions = $derived<SelectOption[]>([
		{ value: 'string', label: t('scripts.parameter.kind.string') },
		{ value: 'secret_ref', label: t('scripts.parameter.kind.secret_ref') }
	]);
	const secretOptions = $derived<SelectOption[]>([
		{ value: '', label: t('scripts.parameter.selectSecret') },
		...secrets.map((secret) => ({ value: secret.id, label: secret.name }))
	]);

	const title = $derived(script.name.trim() || t('scripts.number', { number }));
	const bodyBytes = $derived(byteLength(script.body));
	const bodyError = $derived(
		error('body') ?? (bodyBytes > MAX_BODY_BYTES ? t('scripts.bodyTooLong') : undefined)
	);
	const invalid = $derived(Boolean(error('name') || error('body') || error('env')));

	function addParameter(): void {
		script.env = [...script.env, { name: '', kind: 'string', value: '' }];
	}

	function removeParameter(index: number): void {
		script.env = script.env.filter((_, position) => position !== index);
	}

	function changeKind(index: number, kind: string): void {
		script.env[index].kind = kind as ScriptEnvKind;
		script.env[index].value = '';
	}
</script>

<article class="card" class:card--invalid={invalid} data-testid="script-card">
	<header class="card__head">
		<Button
			variant="ghost"
			size="sm"
			icon={script.open ? 'chevron-down' : 'chevron-right'}
			aria-label={t(script.open ? 'scripts.hide' : 'scripts.show')}
			aria-expanded={script.open}
			onclick={() => (script.open = !script.open)}
		/>
		<span class="card__number">{number}</span>
		<span class="card__title">{title}</span>
		<Badge tone="neutral">{t(`scripts.interpreter.${script.interpreter}` as MessageKey)}</Badge>
		{#if group === 'after'}
			<Badge tone="info">{t(`profiles.hooks.phase.${script.phase}` as MessageKey)}</Badge>
		{/if}
		{#if script.failurePolicy === 'continue'}
			<Badge tone="warning">{t('scripts.policy.continue')}</Badge>
		{/if}
		{#if !readonly}
			<span class="card__tools">
				<Button
					variant="ghost"
					size="sm"
					icon="chevron-up"
					aria-label={t('scripts.moveUp')}
					disabled={!canUp}
					onclick={onUp}
				/>
				<Button
					variant="ghost"
					size="sm"
					icon="chevron-down"
					aria-label={t('scripts.moveDown')}
					disabled={!canDown}
					onclick={onDown}
				/>
				<Button
					variant="ghost"
					size="sm"
					icon="trash"
					aria-label={t('scripts.remove')}
					onclick={onRemove}
				/>
			</span>
		{/if}
	</header>
	{#if script.open}
		<div class="card__body">
			<div class="grid">
				<TextField
					id={`script-${script.key}-name`}
					label={t('scripts.name')}
					error={error('name')}
					bind:value={script.name}
					{readonly}
					required
				/>
				<Select
					id={`script-${script.key}-interpreter`}
					label={t('scripts.interpreter')}
					options={interpreterOptions}
					error={error('interpreter')}
					bind:value={script.interpreter}
					disabled={readonly}
				/>
			</div>
			{#if group === 'after'}
				<Select
					id={`script-${script.key}-phase`}
					label={t('scripts.phase')}
					options={phaseOptions}
					error={error('phase')}
					bind:value={script.phase}
					disabled={readonly}
				/>
			{/if}
			<CodeEditor
				id={`script-${script.key}-body`}
				label={t('scripts.body')}
				help={t('scripts.bodyHelp')}
				error={bodyError}
				bind:value={script.body}
				{readonly}
				required
			/>
			<div class="grid">
				<TextField
					id={`script-${script.key}-timeout`}
					type="number"
					label={t('scripts.timeout')}
					help={t('scripts.timeoutHelp')}
					error={error('timeout_seconds')}
					bind:value={script.timeoutSeconds}
					{readonly}
				/>
				<Select
					id={`script-${script.key}-policy`}
					label={t('scripts.failurePolicy')}
					options={policyOptions}
					bind:value={script.failurePolicy}
					disabled={readonly}
				/>
			</div>
			<section class="params">
				<h4 class="params__title">{t('scripts.parameters')}</h4>
				<p class="params__help">{t('scripts.parametersHelp')}</p>
				{#each script.env as variable, index (index)}
					<div class="param">
						<TextField
							label={t('scripts.parameter.name')}
							mono
							error={error(`env[${index}].name`)}
							bind:value={variable.name}
							{readonly}
						/>
						<Select
							label={t('scripts.parameter.kind')}
							options={kindOptions}
							value={variable.kind}
							onchange={(event) => changeKind(index, event.currentTarget.value)}
							disabled={readonly}
						/>
						{#if variable.kind === 'secret_ref'}
							<div class="param__secret">
								<Select
									label={t('scripts.parameter.secret')}
									options={secretOptions}
									error={error(`env[${index}].value`)}
									bind:value={variable.value}
									disabled={readonly}
								/>
								{#if !readonly}
									<Button variant="ghost" icon="plus" onclick={() => onNewSecret(index)}>
										{t('scripts.secret.new')}
									</Button>
									<a class="param__manage" href={resolve('/(app)/settings/access/secrets')}>
										{t('secrets.manage')}
									</a>
								{/if}
							</div>
						{:else}
							<TextField
								label={t('scripts.parameter.value')}
								mono
								error={error(`env[${index}].value`)}
								bind:value={variable.value}
								{readonly}
							/>
						{/if}
						{#if !readonly}
							<Button
								variant="ghost"
								icon="trash"
								aria-label={t('scripts.parameter.remove')}
								onclick={() => removeParameter(index)}
							/>
						{/if}
					</div>
				{:else}
					<p class="params__help">{t('scripts.parametersEmpty')}</p>
				{/each}
				{#if !readonly}
					<div>
						<Button variant="ghost" icon="plus" onclick={addParameter}>
							{t('scripts.parameter.add')}
						</Button>
					</div>
				{/if}
			</section>
		</div>
	{/if}
</article>

<style>
	.card {
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-canvas);
	}

	.card--invalid {
		border-color: var(--rh-color-danger-text);
	}

	.card__head {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
		padding: var(--rh-space-2) var(--rh-space-3);
	}

	.card__number {
		flex: none;
		min-width: 20px;
		color: var(--rh-color-text-subtle);
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-sm);
		text-align: center;
	}

	.card__title {
		min-width: 0;
		overflow: hidden;
		font-weight: var(--rh-font-weight-semibold);
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.card__tools {
		display: flex;
		gap: var(--rh-space-1);
		margin-inline-start: auto;
	}

	.card__body {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
		padding: var(--rh-space-3) var(--rh-space-4) var(--rh-space-4);
		border-top: var(--rh-border-width) solid var(--rh-color-border);
	}

	.grid {
		display: grid;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		gap: var(--rh-space-3);
	}

	.params {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-2);
	}

	.params__title {
		margin: 0;
		font-size: var(--rh-font-size-sm);
		font-weight: var(--rh-font-weight-semibold);
	}

	.params__help {
		margin: 0;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.param {
		display: grid;
		grid-template-columns: minmax(0, 1.2fr) 110px minmax(0, 1.6fr) auto;
		align-items: end;
		gap: var(--rh-space-2);
	}

	.param__manage {
		white-space: nowrap;
		font-size: var(--rh-font-size-sm);
	}

	.param__secret {
		display: flex;
		align-items: end;
		gap: var(--rh-space-2);
	}

	.param__secret :global(.rh-field) {
		flex: 1;
	}
</style>
