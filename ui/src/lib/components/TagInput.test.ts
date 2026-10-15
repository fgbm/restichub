/*
 * The tag field turns typed text into chips: a comma, a space or Enter ends a tag (FR-PF-01).
 */
import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import TagInput from './TagInput.svelte';

function setup(value: string[] = []) {
	const user = userEvent.setup();
	render(TagInput, { value, label: 'Tags', id: 'tags' });
	return { user, input: screen.getByLabelText('Tags') as HTMLInputElement };
}

function chips(): string[] {
	return screen
		.queryAllByRole('button', { name: /^Remove the tag / })
		.map((button) => button.getAttribute('aria-label')!.replace('Remove the tag ', ''));
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
		await user.click(screen.getByRole('button', { name: 'Remove the tag a' }));
		expect(chips()).toEqual(['b']);
	});

	it('removes the last chip on Backspace in an empty field', async () => {
		const { user, input } = setup(['a', 'b']);
		await user.click(input);
		await user.keyboard('{Backspace}');
		expect(chips()).toEqual(['a']);
	});
});
