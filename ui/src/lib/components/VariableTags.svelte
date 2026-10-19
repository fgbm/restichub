<script lang="ts">
	import type { VariableOption, VariableSource } from '#lib/variables.ts';
	import { caretExpression, insertVariable, variableGroups } from '#lib/variables.ts';
	import VariableList from './VariableList.svelte';
	import TagInput from './ui/TagInput.svelte';

	let {
		id,
		label,
		help,
		error,
		placeholder,
		max,
		value = $bindable([]),
		parameters,
		labels,
		ondeclare
	}: {
		id?: string;
		label?: string;
		help?: string;
		error?: string;
		placeholder?: string;
		max?: number;
		value?: string[];
		parameters: VariableSource['parameters'];
		labels: VariableSource['labels'];
		ondeclare: (name: string) => void;
	} = $props();

	const uid = $props.id();
	const fieldId = $derived(id ?? `rh-variable-tags-${uid}`);
	const listId = $derived(`${fieldId}-list`);

	let wrapper = $state<HTMLDivElement | null>(null);
	let popover = $state<HTMLDivElement | null>(null);
	let input = $state<HTMLInputElement | null>(null);
	let open = $state(false);
	let typed = $state('');
	let start = $state(0);
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

	$effect(() => {
		void top;
		if (popover) popover.style.top = `${top}px`;
	});

	function firstEnabled(list: readonly VariableOption[]): number {
		return list.findIndex((option) => !option.disabled);
	}

	// The tags field is one row, so the list opens below the whole field, as for a one-line input.
	function fieldBottom(element: HTMLInputElement): number {
		const box = element.closest('.tags') ?? element;
		const outer = wrapper?.getBoundingClientRect();
		if (!outer) return 0;
		return box.getBoundingClientRect().bottom - outer.top + 6;
	}

	function update(element: HTMLInputElement): void {
		input = element;
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
		top = fieldBottom(element);
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
		const element = input;
		if (!element || option.disabled) return;
		const inserted = option.partial ? option.expression : `${option.expression}}}`;
		insertVariable(element, start + 2, element.selectionStart ?? start + 2, inserted);
		element.dispatchEvent(new Event('input', { bubbles: true }));
	}

	// A separator still ends the tag: the list closes and the tag input adds the value.
	function onkeydown(event: KeyboardEvent): void {
		const element = event.target as HTMLInputElement;
		if (!open) {
			if (event.key === 'ArrowDown' && dismissed) {
				const context = caretExpression(element.value, element.selectionStart ?? 0);
				if (context) {
					dismissed = false;
					update(element);
					if (open) event.preventDefault();
				}
			}
			return;
		}
		if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
			event.preventDefault();
			event.stopPropagation();
			move(event.key === 'ArrowDown' ? 1 : -1);
		} else if (event.key === 'Enter') {
			// While the list is open, Enter belongs to the list and not to the tag input.
			if (activeOption && !activeOption.disabled) {
				event.preventDefault();
				event.stopPropagation();
				choose(activeOption);
			}
		} else if (event.key === 'Escape') {
			event.preventDefault();
			event.stopPropagation();
			close();
		} else if (event.key === ' ' || event.key === ',') {
			open = false;
		}
	}

	function oninput(event: Event): void {
		update(event.target as HTMLInputElement);
	}

	function onfocusout(event: FocusEvent): void {
		const next = event.relatedTarget as Node | null;
		if (next && wrapper?.contains(next)) return;
		open = false;
	}

	// The tag input writes its text as a tag when it loses the focus, so a click in the list must
	// not move the focus.
	function declare(name: string): void {
		open = false;
		dismissed = true;
		ondeclare(name);
	}
</script>

<div
	class="variable-tags"
	bind:this={wrapper}
	onkeydowncapture={onkeydown}
	oninputcapture={oninput}
	onfocusoutcapture={onfocusout}
>
	<TagInput
		id={fieldId}
		{label}
		{help}
		{error}
		{placeholder}
		{max}
		bind:value
		listbox={{ id: listId, expanded: open, activeId }}
	/>
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

<style>
	.variable-tags {
		position: relative;
	}

	.popover {
		position: absolute;
		right: 0;
		left: 0;
	}
</style>
