<script lang="ts">
	import type { VariableOption, VariableSource } from '#lib/variables.ts';
	import { caretExpression, insertVariable, variableGroups } from '#lib/variables.ts';
	import VariableList from './VariableList.svelte';
	import { Field } from './ui/index.ts';

	let {
		id,
		label,
		help,
		error,
		value = $bindable(''),
		rows = 2,
		parameters,
		labels,
		ondeclare
	}: {
		id?: string;
		label?: string;
		help?: string;
		error?: string;
		value?: string;
		rows?: number;
		parameters: VariableSource['parameters'];
		labels: VariableSource['labels'];
		ondeclare: (name: string) => void;
	} = $props();

	const uid = $props.id();
	const fieldId = $derived(id ?? `rh-variable-${uid}`);
	const listId = $derived(`${fieldId}-list`);

	let field = $state<HTMLTextAreaElement | null>(null);
	let popover = $state<HTMLDivElement | null>(null);
	let open = $state(false);
	let typed = $state('');
	/** The index of the first `{` of the expression that the list follows. */
	let start = $state(0);
	/** Escape closed the list for this expression; the next typed text does not reopen it. */
	let dismissed = $state(false);
	let active = $state(-1);
	let top = $state(0);

	const source = $derived({ parameters, labels });
	const groups = $derived(open ? variableGroups(typed, source) : []);
	const options = $derived(groups.flatMap((group) => group.options));
	const activeOption = $derived(options[active] ?? null);
	const activeId = $derived(activeOption ? `${listId}-${activeOption.expression}` : undefined);
	const total = $derived(
		variableGroups('', source).reduce((count, group) => count + group.options.length, 0)
	);

	// The popover hangs at the line of the caret. CSS cannot read the caret, and a style attribute
	// needs `unsafe-inline`, so the node takes its place (ADR-0027).
	$effect(() => {
		void top;
		if (popover) popover.style.top = `${top}px`;
	});

	function firstEnabled(list: readonly VariableOption[]): number {
		return list.findIndex((option) => !option.disabled);
	}

	function lineTop(element: HTMLTextAreaElement, caret: number): number {
		const style = getComputedStyle(element);
		const lineHeight = parseFloat(style.lineHeight) || 20;
		const border = parseFloat(style.borderTopWidth) || 0;
		const padding = parseFloat(style.paddingTop) || 0;
		const line = element.value.slice(0, caret).split('\n').length - 1;
		const edge = element.clientHeight - border;
		return Math.min(
			Math.max(border + padding + (line + 1) * lineHeight - element.scrollTop, border),
			edge
		);
	}

	function update(element: HTMLTextAreaElement): void {
		const caret = element.selectionStart ?? 0;
		const context = caretExpression(element.value, caret);
		if (!context) {
			dismissed = false;
			open = false;
			return;
		}
		if (dismissed && context.typed !== '') {
			open = false;
			return;
		}
		start = context.start;
		typed = context.typed;
		open = true;
		active = firstEnabled(variableGroups(typed, source).flatMap((group) => group.options));
		top = lineTop(element, caret);
	}

	function move(step: number): void {
		for (let index = active + step; index >= 0 && index < options.length; index += step) {
			if (!options[index].disabled) {
				active = index;
				return;
			}
		}
	}

	function close(): void {
		open = false;
		dismissed = true;
	}

	function choose(option: VariableOption): void {
		const element = field;
		if (!element || option.disabled) return;
		const inserted = option.partial ? option.expression : `${option.expression}}}`;
		insertVariable(element, start + 2, element.selectionStart ?? start + 2, inserted);
		// One input event keeps the draft, the summary and the "Used in" column in one step.
		element.dispatchEvent(new Event('input', { bubbles: true }));
	}

	function oninput(event: Event): void {
		update(event.currentTarget as HTMLTextAreaElement);
	}

	function onkeydown(event: KeyboardEvent): void {
		const element = event.currentTarget as HTMLTextAreaElement;
		if (!open) {
			if (event.key === 'ArrowDown') {
				const context = caretExpression(element.value, element.selectionStart ?? 0);
				if (context && dismissed) {
					dismissed = false;
					update(element);
					if (open) event.preventDefault();
				}
			}
			return;
		}
		if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
			event.preventDefault();
			move(event.key === 'ArrowDown' ? 1 : -1);
		} else if (event.key === 'Enter') {
			if (activeOption && !activeOption.disabled) {
				event.preventDefault();
				choose(activeOption);
			}
		} else if (event.key === 'Escape') {
			event.preventDefault();
			close();
		}
	}

	// A click or a cursor key moves the caret to another line, and the list follows it. The keys of
	// the list keep the highlight, so they do not move the list to the caret again.
	function oncursor(event: Event): void {
		update(event.currentTarget as HTMLTextAreaElement);
	}

	function onkeyup(event: KeyboardEvent): void {
		if (['ArrowDown', 'ArrowUp', 'Enter', 'Escape'].includes(event.key)) return;
		oncursor(event);
	}

	// The list belongs to the field that types the text, so another field closes it. The button
	// "Declare" inside the list keeps it open until its click runs.
	function onblur(event: FocusEvent): void {
		const next = event.relatedTarget as Node | null;
		if (next && popover?.contains(next)) return;
		open = false;
		dismissed = false;
	}

	function declare(name: string): void {
		open = false;
		dismissed = true;
		ondeclare(name);
	}
</script>

<Field {label} {help} {error} for={fieldId}>
	<div class="field">
		<textarea
			bind:this={field}
			id={fieldId}
			class="rh-textarea rh-input--mono"
			{rows}
			bind:value
			aria-autocomplete="list"
			aria-expanded={open}
			aria-controls={open ? listId : undefined}
			aria-activedescendant={activeId}
			{oninput}
			{onkeydown}
			onclick={oncursor}
			{onkeyup}
			{onblur}></textarea>
		{#if open}
			<div class="popover" bind:this={popover}>
				<VariableList
					id={listId}
					{groups}
					{activeOption}
					{typed}
					{total}
					onpick={choose}
					ondeclare={declare}
				/>
			</div>
		{/if}
	</div>
</Field>

<style>
	.field {
		position: relative;
	}

	.popover {
		/* `lineTop` sets the row of the caret; the popover covers the rows below it. */
		position: absolute;
		right: 0;
		left: 0;
		margin-top: 2px;
	}
</style>
