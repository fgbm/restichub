/*
 * The tag input turns typed text into chips: a separator or Enter ends a value (FR-PF-01). A value
 * that fails the check stays in the field with the reason.
 */
import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import TagInput from './TagInput.svelte';

function setup(value: string[] = [], props: Record<string, unknown> = {}) {
	const user = userEvent.setup();
	render(TagInput, { value, label: 'Tags', id: 'tags', ...props });
	return { user, input: screen.getByLabelText('Tags') as HTMLInputElement };
}

function chips(): string[] {
	return screen
		.queryAllByRole('button', { name: /^Remove / })
		.map((button) => button.getAttribute('aria-label')!.replace('Remove ', ''));
}

describe('TagInput', () => {
	it('makes a chip when a comma ends the text', async () => {
		const { user, input } = setup();
		await user.type(input, 'web,');
		expect(chips()).toEqual(['web']);
		expect(input).toHaveValue('');
	});

	it('makes a chip when a space ends the text', async () => {
		const { user, input } = setup();
		await user.type(input, 'web nightly ');
		expect(chips()).toEqual(['web', 'nightly']);
	});

	it('keeps the text after the last separator in the field', async () => {
		const { user, input } = setup();
		await user.type(input, 'a, b, c');
		expect(chips()).toEqual(['a', 'b']);
		expect(input).toHaveValue('c');
	});

	it('makes a chip on Enter without sending a form', async () => {
		const { user, input } = setup();
		await user.type(input, 'db{Enter}');
		expect(chips()).toEqual(['db']);
	});

	it('splits pasted text into chips', async () => {
		const { user, input } = setup();
		await user.click(input);
		await user.paste('one, two three');
		await user.tab();
		expect(chips()).toEqual(['one', 'two', 'three']);
	});

	it('makes a chip when the field loses focus', async () => {
		const { user, input } = setup();
		await user.type(input, 'last');
		await user.tab();
		expect(chips()).toEqual(['last']);
	});

	it('does not add a tag twice', async () => {
		const { user, input } = setup(['web']);
		await user.type(input, 'web, db,');
		expect(chips()).toEqual(['web', 'db']);
	});

	it('removes a chip with its button', async () => {
		const { user } = setup(['a', 'b']);
		await user.click(screen.getByRole('button', { name: 'Remove a' }));
		expect(chips()).toEqual(['b']);
	});

	it('removes the last chip on Backspace in an empty field', async () => {
		const { user, input } = setup(['a', 'b']);
		await user.click(input);
		await user.keyboard('{Backspace}');
		expect(chips()).toEqual(['a']);
	});

	it('adds each value of a paste such as "a, b"', async () => {
		const { user, input } = setup();
		await user.click(input);
		await user.paste('a, b,');
		expect(chips()).toEqual(['a', 'b']);
		expect(input).toHaveValue('');
	});

	it('keeps a value that fails the check in the field with the reason', async () => {
		const { user, input } = setup([], {
			check: (value: string) => (/^[a-z]+$/.test(value) ? null : 'Only a-z.')
		});
		await user.click(input);
		await user.paste('ok, Bad1, fine,');
		expect(chips()).toEqual(['ok', 'fine']);
		expect(input).toHaveValue('Bad1, ');
		expect(input).toHaveAttribute('aria-invalid', 'true');
		expect(screen.getByRole('alert')).toHaveTextContent('Bad1: Only a-z.');
		await user.clear(input);
		expect(screen.queryByRole('alert')).not.toBeInTheDocument();
	});

	it('does not join the next value to a rejected one', async () => {
		const { user, input } = setup([], {
			check: (value: string) => (/^[a-z]+$/.test(value) ? null : 'Only a-z.')
		});
		await user.type(input, 'Bad1, ok');
		expect(input).toHaveValue('Bad1, ok');
		await user.tab();
		expect(chips()).toEqual(['ok']);
		expect(input).toHaveValue('Bad1');
	});

	it('refuses a value above the length limit', async () => {
		const { user, input } = setup([], { maxLength: 3 });
		await user.type(input, 'abcd{Enter}');
		expect(chips()).toEqual([]);
		expect(screen.getByRole('alert')).toHaveTextContent('abcd: At most 3 characters.');
	});

	it('refuses a value above the count limit', async () => {
		const { user, input } = setup(['a'], { max: 2 });
		await user.type(input, 'b, c,');
		expect(chips()).toEqual(['a', 'b']);
		expect(input).toHaveValue('c,');
		expect(screen.getByRole('alert')).toHaveTextContent('c: At most 2 values.');
	});

	it('splits on the given separator only', async () => {
		const { user, input } = setup([], { separator: /[;\n]+/ });
		await user.type(input, 'a b;c,d;');
		expect(chips()).toEqual(['a b', 'c,d']);
	});

	it('lets Enter in an empty field send the form', async () => {
		const user = userEvent.setup();
		const submit = vi.fn((event: SubmitEvent) => event.preventDefault());
		const form = document.createElement('form');
		form.addEventListener('submit', submit);
		document.body.append(form);
		render(TagInput, { target: form, props: { value: [], label: 'Tags', id: 'tags' } });
		await user.type(screen.getByLabelText('Tags'), 'a{Enter}');
		expect(submit).not.toHaveBeenCalled();
		await user.keyboard('{Enter}');
		expect(submit).toHaveBeenCalledTimes(1);
		form.remove();
	});
});
