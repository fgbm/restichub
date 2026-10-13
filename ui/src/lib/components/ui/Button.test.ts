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

	it('renders a leading icon with the label', () => {
		render(Button, { props: { icon: 'i-plus', children: label('Add host') } });
		const button = screen.getByRole('button', { name: 'Add host' });
		expect(button).not.toHaveClass('btn--icon');
		expect(button.querySelector('svg.icon use')).toHaveAttribute('href', '/icons.svg#i-plus');
	});

	it('holds the icon only and takes the name from aria-label', () => {
		render(Button, { props: { icon: 'i-refresh', 'aria-label': 'Refresh' } });
		const button = screen.getByRole('button', { name: 'Refresh' });
		expect(button).toHaveClass('btn--icon');
	});
});
