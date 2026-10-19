/*
 * The list of variables in a text field: the filter, the keys, the insertion with the closing
 * braces and the button "Declare" (FR-PF-03, FR-PF-10, AC-PF-09).
 */
import { fireEvent, render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import VariableTextarea from './VariableTextarea.svelte';
import type { VariableSource } from '#lib/variables.ts';

const source: VariableSource = {
	parameters: [
		{ name: 'pg_password', kind: 'secret', description: 'Backup password' },
		{ name: 'pg_dir', kind: 'text', description: 'Directory of the dumps' },
		{ name: 'pg_host', kind: 'text', description: 'Address of the database' }
	],
	labels: [
		{ key: 'env', value: 'prod' },
		{ key: 'role', value: 'db' }
	]
};

function setup(value = '/srv/') {
	const user = userEvent.setup();
	const ondeclare = vi.fn();
	render(VariableTextarea, {
		label: 'All systems',
		parameters: source.parameters,
		labels: source.labels,
		ondeclare: ondeclare,
		value
	});
	const field = screen.getByLabelText('All systems') as HTMLTextAreaElement;
	return { user, field, ondeclare };
}

/** Adds the text at the end of the field, as the keyboard does. */
async function typeAtEnd(
	user: ReturnType<typeof userEvent.setup>,
	field: HTMLTextAreaElement,
	text: string
): Promise<void> {
	await user.click(field);
	field.value += text;
	field.setSelectionRange(field.value.length, field.value.length);
	await fireEvent.input(field);
}

const listbox = () => screen.queryByRole('listbox', { name: 'Variables' });

describe('the list of variables in a text field', () => {
	it('fr_pf_03_opens_after_two_braces_and_follows_the_caret', async () => {
		const { user, field } = setup();
		await user.click(field);
		field.setSelectionRange(field.value.length, field.value.length);
		// user-event writes a literal brace as `{{`.
		await user.keyboard('{{{{pg');

		expect(field.value).toBe('/srv/{{pg');
		expect(listbox()).toBeInTheDocument();
		expect(field).toHaveAttribute('aria-expanded', 'true');
		expect(field).toHaveAttribute('aria-controls', listbox()?.id);
		expect(screen.getAllByRole('option').map((option) => option.textContent?.trim())).toEqual([
			expect.stringContaining('pg_password'),
			expect.stringContaining('pg_dir'),
			expect.stringContaining('pg_host')
		]);
		expect(field.getAttribute('aria-activedescendant')).toBe(screen.getAllByRole('option')[1].id);
	});

	it('fr_pf_10_disables_the_secret_and_the_arrows_skip_it', async () => {
		const { user, field } = setup();
		await typeAtEnd(user, field, '{{pg');
		const secret = screen.getByRole('option', { name: /pg_password/ });
		expect(secret).toHaveAttribute('aria-disabled', 'true');
		// The reason of the refusal takes the place of the description, and it stays described.
		expect(secret).toHaveTextContent('A secret cannot go into a path or a tag.');
		expect(secret).not.toHaveTextContent('Backup password');
		expect(secret).toHaveAccessibleDescription('A secret cannot go into a path or a tag.');

		// The first enabled row is pg_dir; the next one is pg_host, and the secret never takes it.
		await user.keyboard('{ArrowDown}{Enter}');
		expect(field.value).toBe('/srv/{{param.pg_host}}');
	});

	it('fr_pf_03_inserts_the_closing_braces_and_puts_the_caret_after_them', async () => {
		const { user, field } = setup('/var/backups/');
		await typeAtEnd(user, field, '{{pg');
		await user.keyboard('{Enter}');

		expect(field.value).toBe('/var/backups/{{param.pg_dir}}');
		expect(field.selectionStart).toBe(field.value.length);
		expect(listbox()).not.toBeInTheDocument();
	});

	it('fr_pf_03_closes_on_escape_and_keeps_the_text', async () => {
		const { user, field } = setup();
		await typeAtEnd(user, field, '{{pg');
		await user.keyboard('{Escape}');

		expect(field.value).toBe('/srv/{{pg');
		expect(listbox()).not.toBeInTheDocument();
		expect(field).toHaveAttribute('aria-expanded', 'false');
	});

	it('fr_pf_03_writes_the_label_prefix_and_opens_the_keys', async () => {
		const { user, field } = setup();
		await typeAtEnd(user, field, '{{agent.labe');
		await user.keyboard('{Enter}');

		expect(field.value).toBe('/srv/{{agent.label.');
		expect(screen.getAllByRole('option').map((option) => option.textContent)).toEqual([
			expect.stringContaining('agent.label.env'),
			expect.stringContaining('agent.label.role')
		]);
	});

	it('fr_pf_09_declares_a_typed_name_only_when_the_server_accepts_it', async () => {
		const { user, field, ondeclare } = setup();
		await typeAtEnd(user, field, '{{zz');

		await user.click(screen.getByRole('button', { name: 'Declare' }));
		expect(ondeclare).toHaveBeenCalledWith('zz');
	});

	it('fr_pf_09_shows_no_button_for_a_name_that_the_server_refuses', async () => {
		const { user, field } = setup();
		await typeAtEnd(user, field, '{{Zz-');

		expect(screen.getByText(/No matches/)).toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Declare' })).not.toBeInTheDocument();
	});

	it('fr_pf_03_the_list_leaves_the_field_and_keeps_the_full_opacity_of_the_page', async () => {
		const user = userEvent.setup();
		const { container } = render(VariableTextarea, {
			label: 'All systems',
			parameters: source.parameters,
			labels: source.labels,
			ondeclare: vi.fn(),
			value: '/srv/'
		});
		// A switched-off section dims its fields with the opacity of the body.
		container.style.opacity = '0.55';
		const field = screen.getByLabelText('All systems') as HTMLTextAreaElement;
		await typeAtEnd(user, field, '{{pg');

		const list = listbox() as HTMLElement;
		expect(container.contains(list)).toBe(false);
		expect(list.closest('[style*="opacity"]')).toBeNull();
		// The list is a child of the body, so a later section cannot paint over it.
		expect(list.parentElement?.parentElement?.parentElement).toBe(document.body);
	});

	it('fr_pf_03_a_pointer_down_in_the_field_keeps_the_list_open', async () => {
		const { user, field } = setup();
		await typeAtEnd(user, field, '{{pg');

		await fireEvent.pointerDown(field);
		expect(listbox()).toBeInTheDocument();
	});
});
