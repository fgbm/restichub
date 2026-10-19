<script lang="ts">
	import Field from './Field.svelte';
	import Icon from './Icon.svelte';
	import { t } from '#lib/i18n/index.ts';

	let {
		value = $bindable([]),
		label,
		ariaLabel,
		help,
		error,
		id,
		placeholder,
		mono = false,
		max,
		maxLength,
		check,
		separator = /[\s,]+/,
		listbox
	}: {
		value?: string[];
		label?: string;
		/** The name of the input when a heading nearby already shows it. */
		ariaLabel?: string;
		help?: string;
		error?: string;
		id?: string;
		placeholder?: string;
		mono?: boolean;
		max?: number;
		maxLength?: number;
		/** Returns the reason why a value is not valid, or null. */
		check?: (value: string) => string | null;
		separator?: RegExp;
		/**
		 * A list below the field that the typed text drives. The field keeps its keys while the list
		 * is open; the parent moves the highlight.
		 */
		listbox?: { id: string; expanded: boolean; activeId?: string };
	} = $props();

	const uid = $props.id();
	const inputId = $derived(id ?? `rh-tags-${uid}`);
	const helpId = $derived(`${inputId}-help`);
	const errorId = $derived(`${inputId}-error`);

	let text = $state('');
	let reason = $state<string | null>(null);
	const shownError = $derived(error ?? reason ?? undefined);

	function reasonOf(item: string, count: number): string | null {
		if (maxLength !== undefined && [...item].length > maxLength) {
			return t('tagInput.tooLong', { max: maxLength });
		}
		const own = check?.(item);
		if (own) return own;
		if (max !== undefined && count >= max) return t('tagInput.tooMany', { max });
		return null;
	}

	/** Adds the valid new values and returns the rejected ones, which stay in the field. */
	function take(items: string[]): string[] {
		const next = [...value];
		const rejected: string[] = [];
		reason = null;
		for (const item of items) {
			if (next.includes(item)) continue;
			const why = reasonOf(item, next.length);
			if (why) {
				rejected.push(item);
				reason ??= t('tagInput.invalid', { value: item, reason: why });
			} else {
				next.push(item);
			}
		}
		if (next.length !== value.length) value = next;
		return rejected;
	}

	function clean(items: string[]): string[] {
		return items.map((item) => item.trim()).filter((item) => item !== '');
	}

	function commit(): void {
		const joint = text.match(separator)?.[0] ?? '';
		text = take(clean(text.split(separator))).join(joint);
	}

	// A separator ends a value. What follows the last separator stays in the field, after the
	// rejected values and their separator, so the next value does not join a rejected one.
	function onInput(): void {
		const parts = text.split(separator);
		if (parts.length < 2) {
			reason = null;
			return;
		}
		const joint = text.match(separator)?.[0] ?? '';
		const rest = parts.pop() ?? '';
		const rejected = take(clean(parts));
		text = rejected.length > 0 ? [...rejected, rest].join(joint) : rest;
	}

	function onKeydown(event: KeyboardEvent): void {
		if (event.key === 'Enter') {
			// An empty field lets Enter send the form.
			if (text.trim() === '') return;
			event.preventDefault();
			commit();
		} else if (event.key === 'Backspace' && text === '' && value.length > 0) {
			value = value.slice(0, -1);
			reason = null;
		}
	}

	function remove(item: string): void {
		value = value.filter((entry) => entry !== item);
		reason = null;
	}
</script>

<Field {label} {help} error={shownError} for={inputId} {helpId} {errorId}>
	<div class="tags" class:tags--invalid={Boolean(shownError)}>
		{#each value as item (item)}
			<span class="chip">
				<span class="chip__text" class:mono>{item}</span>
				<button
					type="button"
					class="chip__remove"
					aria-label={t('tagInput.remove', { value: item })}
					onclick={() => remove(item)}
				>
					<Icon name="x" size="xs" />
				</button>
			</span>
		{/each}
		<input
			id={inputId}
			class="tags__input"
			class:mono
			autocomplete="off"
			aria-label={label ? undefined : ariaLabel}
			placeholder={value.length === 0 ? (placeholder ?? t('tagInput.placeholder')) : undefined}
			aria-invalid={shownError ? 'true' : undefined}
			aria-describedby={shownError ? errorId : help ? helpId : undefined}
			aria-autocomplete={listbox ? 'list' : undefined}
			aria-expanded={listbox ? listbox.expanded : undefined}
			aria-controls={listbox?.expanded ? listbox.id : undefined}
			aria-activedescendant={listbox?.expanded ? listbox.activeId : undefined}
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
		max-width: 100%;
		padding: 0 2px 0 var(--rh-space-2);
		border: var(--rh-border-width) solid var(--rh-color-accent-subtle-border);
		border-radius: var(--rh-radius-pill);
		background-color: var(--rh-color-accent-subtle);
		color: var(--rh-color-accent-subtle-text);
		font-size: var(--rh-font-size-sm);
		line-height: 1.6;
	}

	.chip__text {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
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

	.chip__remove:focus-visible {
		outline: 2px solid var(--rh-color-focus);
		outline-offset: 1px;
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

	.mono {
		font-family: var(--rh-font-mono);
	}
</style>
