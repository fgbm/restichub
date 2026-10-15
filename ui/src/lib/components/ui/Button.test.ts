import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { createRawSnippet, type Snippet } from 'svelte';
import { describe, expect, it, vi } from 'vitest';
import Button from './Button.svelte';

function label(value: string): Snippet {
	return createRawSnippet(() => ({ render: () => `<span>${value}</span>` }));
}

describe('Button', () => {
	it('renders the label and the variant class', () => {
		render(Button, { props: { variant: 'primary', children: label('Save') } });
		const button = screen.getByRole('button', { name: 'Save' });
		expect(button).toHaveClass('btn--primary');
		expect(button).not.toBeDisabled();
	});

	it('calls onclick', async () => {
		const user = userEvent.setup();
		const onclick = vi.fn();
		render(Button, { props: { onclick, children: label('Run') } });
		await user.click(screen.getByRole('button', { name: 'Run' }));
		expect(onclick).toHaveBeenCalledOnce();
	});

	it('disables the button and marks it busy while loading', () => {
		render(Button, { props: { loading: true, children: label('Save') } });
		const button = screen.getByRole('button', { name: 'Save' });
		expect(button).toBeDisabled();
		expect(button).toHaveAttribute('aria-busy', 'true');
	});

	it('keeps the base classes next to a class of the caller', () => {
		render(Button, { props: { class: 'extra', children: label('Save') } });
		expect(screen.getByRole('button', { name: 'Save' })).toHaveClass(
			'btn',
			'btn--secondary',
			'extra'
		);
	});
});
