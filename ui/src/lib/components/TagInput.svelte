<script lang="ts">
	import Field from './ui/Field.svelte';
	import Icon from './ui/Icon.svelte';
	import { parseTags } from '#lib/profile-form.ts';
	import { t } from '#lib/i18n/index.ts';

	let {
		value = $bindable([]),
		label,
		help,
		error,
		id,
		placeholder
	}: {
		value?: string[];
		label?: string;
		help?: string;
		error?: string;
		id?: string;
		placeholder?: string;
	} = $props();

	const uid = $props.id();
	const inputId = $derived(id ?? `rh-tags-${uid}`);
	const helpId = $derived(`${inputId}-help`);
	const errorId = $derived(`${inputId}-error`);

	let text = $state('');

	function add(tags: string[]): void {
		const next = [...value];
		for (const tag of tags) if (!next.includes(tag)) next.push(tag);
		if (next.length !== value.length) value = next;
	}

	function commit(): void {
		add(parseTags(text));
		text = '';
	}

	// A comma or a space ends a tag. What follows the last separator stays in the field.
	function onInput(): void {
		if (!/[\s,]/.test(text)) return;
		const ended = /[\s,]$/.test(text);
		const parts = parseTags(text);
		const rest = ended ? '' : (parts.pop() ?? '');
		add(parts);
		text = rest;
	}

	function onKeydown(event: KeyboardEvent): void {
		if (event.key === 'Enter') {
			event.preventDefault();
			commit();
		} else if (event.key === 'Backspace' && text === '' && value.length > 0) {
			value = value.slice(0, -1);
		}
	}

	function remove(tag: string): void {
		value = value.filter((item) => item !== tag);
	}
</script>

<Field {label} {help} {error} for={inputId} {helpId} {errorId}>
	<div class="tags" class:tags--invalid={Boolean(error)}>
		{#each value as tag (tag)}
			<span class="chip">
				<span class="chip__text">{tag}</span>
				<button
					type="button"
					class="chip__remove"
					aria-label={t('pe.tags.remove', { tag })}
					onclick={() => remove(tag)}
				>
					<Icon name="x" size="xs" />
				</button>
			</span>
		{/each}
		<input
			id={inputId}
			class="tags__input"
			autocomplete="off"
			placeholder={placeholder ?? t('pe.tags.placeholder')}
			aria-invalid={error ? 'true' : undefined}
			aria-describedby={error ? errorId : help ? helpId : undefined}
			bind:value={text}
			oninput={onInput}
			onkeydown={onKeydown}
			onblur={commit}
		/>
	</div>
</Field>

<style>
	.tags {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--rh-space-1);
		min-height: var(--rh-control-height);
		padding: 3px var(--rh-space-2);
		border: var(--rh-border-width) solid var(--rh-color-border-strong);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-input-bg);
	}

	.tags:focus-within {
		border-color: var(--rh-color-focus);
		box-shadow: 0 0 0 3px var(--rh-color-focus-ring);
	}

	.tags--invalid {
		border-color: var(--rh-color-danger);
	}

	.chip {
		display: inline-flex;
		align-items: center;
		gap: 2px;
		padding: 0 2px 0 var(--rh-space-2);
		border: var(--rh-border-width) solid var(--rh-color-accent-subtle-border);
		border-radius: var(--rh-radius-pill);
		background-color: var(--rh-color-accent-subtle);
		color: var(--rh-color-accent-subtle-text);
		font-size: var(--rh-font-size-sm);
		line-height: 1.6;
	}

	.chip__remove {
		display: inline-flex;
		align-items: center;
		padding: 2px;
		border: 0;
		border-radius: var(--rh-radius-pill);
		background: none;
		color: inherit;
		cursor: pointer;
	}

	.chip__remove:hover {
		background-color: var(--rh-color-hover);
	}

	.tags__input {
		flex: 1;
		min-width: 140px;
		height: 28px;
		padding: 0;
		border: 0;
		background: none;
		color: var(--rh-color-text);
		font: inherit;
		outline: none;
	}

	.tags__input::placeholder {
		color: var(--rh-color-text-subtle);
	}
</style>
