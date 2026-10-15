<script lang="ts">
	import Field from './ui/Field.svelte';
	import { t } from '#lib/i18n/index.ts';

	let {
		value = $bindable(''),
		label,
		help,
		error,
		id,
		readonly = false,
		required = false
	}: {
		value?: string;
		label?: string;
		help?: string;
		error?: string;
		id?: string;
		readonly?: boolean;
		required?: boolean;
	} = $props();

	// The fewest lines that the editor shows.
	const MIN_LINES = 6;

	const uid = $props.id();
	const editorId = $derived(id ?? `rh-code-${uid}`);
	const helpId = $derived(`${editorId}-help`);
	const errorId = $derived(`${editorId}-error`);
	const describedBy = $derived(error ? errorId : help ? helpId : undefined);
	const lineCount = $derived(value.split('\n').length);
	const numbers = $derived(
		Array.from({ length: Math.max(lineCount, MIN_LINES) }, (_, index) => index + 1).join('\n')
	);
</script>

<Field {label} {help} {error} for={editorId} {required} {helpId} {errorId}>
	<div class="code" class:code--invalid={Boolean(error)}>
		<pre class="code__gutter" aria-hidden="true">{numbers}</pre>
		<textarea
			id={editorId}
			class="code__text"
			bind:value
			rows={Math.max(lineCount, MIN_LINES)}
			wrap="off"
			spellcheck="false"
			autocapitalize="off"
			autocomplete="off"
			aria-invalid={error ? 'true' : undefined}
			aria-describedby={describedBy}
			aria-label={label ? undefined : t('scripts.body')}
			{readonly}
			{required}></textarea>
	</div>
	<p class="code__count">{t('scripts.lines', { count: lineCount })}</p>
</Field>

<style>
	.code {
		display: grid;
		grid-template-columns: auto minmax(0, 1fr);
		max-height: 420px;
		overflow: auto;
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-code-bg);
		font-family: var(--rh-font-mono);
		font-size: var(--rh-font-size-sm);
		line-height: 1.6;
	}

	.code:focus-within {
		border-color: var(--rh-color-focus);
		box-shadow: 0 0 0 3px var(--rh-color-focus-ring);
	}

	.code--invalid {
		border-color: var(--rh-color-danger-text);
	}

	.code__gutter {
		margin: 0;
		padding: var(--rh-space-2) var(--rh-space-2) var(--rh-space-2) var(--rh-space-3);
		position: sticky;
		inset-inline-start: 0;
		min-width: 40px;
		border-inline-end: var(--rh-border-width) solid var(--rh-color-border);
		background-color: var(--rh-color-code-bg);
		color: var(--rh-color-text-subtle);
		font: inherit;
		text-align: end;
		user-select: none;
	}

	.code__text {
		display: block;
		box-sizing: border-box;
		min-width: 100%;
		margin: 0;
		padding: var(--rh-space-2) var(--rh-space-3);
		border: 0;
		outline: none;
		background: transparent;
		color: var(--rh-color-text);
		font: inherit;
		line-height: inherit;
		field-sizing: content;
		overflow: hidden;
		resize: none;
		white-space: pre;
		tab-size: 4;
	}

	.code__count {
		margin: var(--rh-space-1) 0 0;
		color: var(--rh-color-text-subtle);
		font-size: var(--rh-font-size-xs);
	}
</style>
