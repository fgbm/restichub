<script lang="ts">
	import { t } from '#lib/i18n/index.ts';
	import { isParameterName, kindLabel } from '#lib/parameters.ts';
	import type { VariableGroup, VariableOption } from '#lib/variables.ts';
	import { variableCount } from '#lib/variables.ts';
	import { Button, Icon } from './ui/index.ts';

	let {
		id,
		groups,
		activeOption = null,
		typed,
		total,
		onpick,
		ondeclare
	}: {
		id: string;
		groups: VariableGroup[];
		/** The option of the highlight; Enter writes it. */
		activeOption?: VariableOption | null;
		typed: string;
		/** The count of all options, for "2 of 9"; the footer counts the hidden secret too. */
		total: number;
		onpick: (option: VariableOption) => void;
		ondeclare: (name: string) => void;
	} = $props();

	const shown = $derived(groups.reduce((count, group) => count + group.options.length, 0));
	const count = $derived(
		typed.trim() === '' ? variableCount(total) : t('recordPicker.shown', { shown, total })
	);
	// The name rule of the server decides the button, so the same rule as the hint below the field.
	const canDeclare = $derived(typed.trim() !== '' && isParameterName(typed));

	let list = $state<HTMLDivElement | null>(null);

	// The list scrolls at 420 px, so the highlight stays in view for the arrow keys.
	$effect(() => {
		if (!activeOption || !list) return;
		list.querySelector('[aria-selected="true"]')?.scrollIntoView?.({ block: 'nearest' });
	});

	function optionId(option: VariableOption): string {
		return `${id}-${option.expression}`;
	}

	function describedBy(option: VariableOption): string | undefined {
		const ids = [
			option.description ? `${optionId(option)}-d` : '',
			option.reason ? `${optionId(option)}-r` : ''
		].filter(Boolean);
		return ids.length > 0 ? ids.join(' ') : undefined;
	}
</script>

<!-- A click in the list keeps the focus in the field that drives it, so the list stays open and a
     tag input does not turn its text into a tag. -->
<div
	class="vars"
	role="listbox"
	{id}
	tabindex="-1"
	aria-label={t('pe.vars.list')}
	onmousedown={(event) => event.preventDefault()}
>
	<div class="vars__body" bind:this={list}>
		{#if shown === 0}
			<div class="vars__none">
				<span>{t('pe.vars.none')}</span>
				{#if canDeclare}
					<Button size="sm" icon="plus" onclick={() => ondeclare(typed.trim())}>
						{t('pe.param.declare')}
					</Button>
				{/if}
			</div>
		{:else}
			{#each groups as group (group.id)}
				<div class="vars__group" role="group" aria-labelledby="{id}-{group.id}">
					<div class="vars__head" id="{id}-{group.id}">{group.label}</div>
					{#each group.options as option (option.expression)}
						<div
							id={optionId(option)}
							role="option"
							class="vars__option"
							class:vars__option--active={option === activeOption}
							class:vars__option--disabled={option.disabled}
							aria-selected={option === activeOption}
							aria-disabled={option.disabled ? 'true' : undefined}
							aria-labelledby={`${optionId(option)}-n`}
							aria-describedby={describedBy(option)}
							tabindex="-1"
							onclick={() => onpick(option)}
							onkeydown={(event) => {
								if (event.key === 'Enter' || event.key === ' ') {
									event.preventDefault();
									onpick(option);
								}
							}}
						>
							<span class="vars__text">
								<span class="vars__name" id={`${optionId(option)}-n`}>
									<span class="vars__mono">{option.label}</span>
									{#if option.kind}
										<span class="vars__kind">
											{#if option.kind === 'secret'}<Icon name="lock" size="xs" />{/if}
											{kindLabel(option.kind)}
										</span>
									{/if}
								</span>
								{#if option.description && !option.reason}
									<span class="vars__desc" id={`${optionId(option)}-d`}>{option.description}</span>
								{/if}
								{#if option.reason}
									<span class="vars__reason" id={`${optionId(option)}-r`}>{option.reason}</span>
								{/if}
							</span>
							{#if option.note}
								<span class="vars__note">{option.note}</span>
							{/if}
						</div>
					{/each}
				</div>
			{/each}
		{/if}
	</div>
	<div class="vars__foot">
		<span role="status">{count}</span>
		<span class="vars__keys">{t('pe.vars.keys')}</span>
	</div>
</div>

<style>
	.vars {
		position: absolute;
		right: 0;
		left: 0;
		/* The popover covers the lines below the caret; the field keeps its height. */
		z-index: var(--rh-z-panel);
		display: flex;
		flex-direction: column;
		max-height: 420px;
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-surface);
		box-shadow: var(--rh-shadow-menu);
	}

	.vars__body {
		display: flex;
		flex-direction: column;
		overflow-y: auto;
	}

	.vars__group {
		display: flex;
		flex-direction: column;
	}

	.vars__head {
		padding: var(--rh-space-2) var(--rh-space-2) var(--rh-space-1);
		color: var(--rh-color-text-subtle);
		font-size: var(--rh-font-size-xs);
		font-weight: var(--rh-font-weight-semibold);
	}

	.vars__option {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: var(--rh-space-3);
		padding: var(--rh-space-1) var(--rh-space-2);
		border-radius: var(--rh-radius-sm);
		cursor: pointer;
	}

	.vars__option--active {
		background-color: var(--rh-color-hover);
	}

	.vars__option--disabled {
		cursor: not-allowed;
	}

	.vars__option--disabled .vars__name,
	.vars__option--disabled .vars__reason {
		opacity: 0.55;
	}

	.vars__text {
		display: flex;
		flex: 1;
		flex-direction: column;
		gap: 2px;
		min-width: 0;
	}

	.vars__name {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
		min-width: 0;
	}

	.vars__mono {
		overflow: hidden;
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-sm);
		font-weight: var(--rh-font-weight-medium);
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.vars__kind {
		display: inline-flex;
		flex: none;
		align-items: center;
		gap: 2px;
		height: 18px;
		padding: 0 var(--rh-space-2);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-pill);
		background-color: var(--rh-color-surface-muted);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-xs);
		font-weight: var(--rh-font-weight-semibold);
	}

	.vars__desc,
	.vars__reason {
		overflow: hidden;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-xs);
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.vars__note {
		flex: none;
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-xs);
		white-space: nowrap;
	}

	.vars__none {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: var(--rh-space-2);
		padding: var(--rh-space-2);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}

	.vars__foot {
		display: flex;
		justify-content: space-between;
		gap: var(--rh-space-3);
		margin-top: auto;
		padding: var(--rh-space-1) var(--rh-space-3);
		border-top: var(--rh-border-width) solid var(--rh-color-border-muted);
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-xs);
	}

	.vars__keys {
		white-space: nowrap;
	}

	@media (max-width: 520px) {
		.vars__foot {
			flex-direction: column;
			gap: 2px;
		}

		.vars__keys {
			white-space: normal;
		}
	}
</style>
