/*
 * The list of variables in the tag input: Enter inserts while the list is open, the second Enter
 * adds the chip, and a separator still ends the tag (FR-PF-03, FR-PF-10).
 */
import { fireEvent, render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import VariableTags from './VariableTags.svelte';
import type { VariableSource } from '#lib/variables.ts';

const source: VariableSource = {
	parameters: [
		{ name: 'pg_password', kind: 'secret', description: 'Backup password' },
		{ name: 'pg_dir', kind: 'text', description: 'Directory of the dumps' },
		{ name: 'pg_host', kind: 'text', description: 'Address of the database' }
	],
	labels: []
};

function setup(value: string[] = []) {
	const user = userEvent.setup();
	const ondeclare = vi.fn();
	render(VariableTags, {
		label: 'Tags',
		parameters: source.parameters,
		labels: source.labels,
		ondeclare,
		value
	});
	const field = screen.getByLabelText('Tags') as HTMLInputElement;
	return { user, field, ondeclare };
}

async function typeAtEnd(
	user: ReturnType<typeof userEvent.setup>,
	field: HTMLInputElement,
	text: string
): Promise<void> {
	await user.click(field);
	field.value += text;
	field.setSelectionRange(field.value.length, field.value.length);
	await fireEvent.input(field);
}

const listbox = () => screen.queryByRole('listbox', { name: 'Variables' });

function chips(): string[] {
	return screen
		.queryAllByRole('button', { name: /^Remove / })
		.map((button) => button.getAttribute('aria-label')!.replace('Remove ', ''));
}

describe('the list of variables in the tag input', () => {
	it('fr_pf_03_inserts_into_the_tag_and_the_second_enter_makes_the_chip', async () => {
		const { user, field } = setup(['prod']);
		await typeAtEnd(user, field, 'db-{{pg');

		expect(listbox()).toBeInTheDocument();
		expect(field).toHaveAttribute('aria-autocomplete', 'list');
		expect(field).toHaveAttribute('aria-expanded', 'true');
		expect(field).toHaveAttribute('aria-controls', listbox()?.id);

		await user.keyboard('{Enter}');
		expect(field.value).toBe('db-{{param.pg_dir}}');
		expect(field.selectionStart).toBe(field.value.length);
		expect(listbox()).not.toBeInTheDocument();
		// The variable is part of one tag and not a chip of its own.
		expect(chips()).toEqual(['prod']);

		await user.keyboard('{Enter}');
		expect(chips()).toEqual(['prod', 'db-{{param.pg_dir}}']);
	});

	it('fr_pf_10_skips_the_secret_with_the_arrows', async () => {
		const { user, field } = setup();
		await typeAtEnd(user, field, '{{pg');

		expect(screen.getByRole('option', { name: /pg_password/ })).toHaveAttribute(
			'aria-disabled',
			'true'
		);
		await user.keyboard('{ArrowDown}{Enter}');
		expect(field.value).toBe('{{param.pg_host}}');
	});

	it('fr_pf_09_declares_the_typed_name_and_adds_no_tag', async () => {
		const { user, field, ondeclare } = setup();
		await typeAtEnd(user, field, 'db-{{zz');

		await user.click(screen.getByRole('button', { name: 'Declare' }));
		expect(ondeclare).toHaveBeenCalledWith('zz');
		// The click in the list does not move the focus, so the tag input keeps its text.
		expect(chips()).toEqual([]);
		expect(field.value).toBe('db-{{zz');
	});

	it('fr_pf_03_closes_the_list_on_a_space_and_adds_the_tag', async () => {
		const { user, field } = setup();
		await typeAtEnd(user, field, 'db-{{pg');
		await user.keyboard(' ');

		expect(listbox()).not.toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Remove db-{{pg' })).toBeInTheDocument();
		expect(field.value).toBe('');
	});
});
