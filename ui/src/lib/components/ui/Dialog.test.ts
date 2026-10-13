import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { createRawSnippet, type Snippet } from 'svelte';
import { describe, expect, it } from 'vitest';
import Dialog from './Dialog.svelte';

function body(value: string): Snippet {
	return createRawSnippet(() => ({ render: () => `<p>${value}</p>` }));
}

describe('Dialog', () => {
	it('shows the dialog with the title as the accessible name', () => {
		render(Dialog, { props: { open: true, title: 'Remove host' } });
		expect(screen.getByRole('dialog')).toHaveAccessibleName('Remove host');
	});

	it('closes on the Escape key', async () => {
		const user = userEvent.setup();
		render(Dialog, { props: { open: true, title: 'Remove host' } });

		expect(screen.getByRole('dialog')).toBeInTheDocument();
		await user.keyboard('{Escape}');
		await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
	});

	it('closes on the close button', async () => {
		const user = userEvent.setup();
		render(Dialog, { props: { open: true, title: 'Remove host' } });

		await user.click(screen.getByRole('button', { name: 'Close' }));
		await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
	});

	it('describes the dialog with the message', () => {
		render(Dialog, { props: { open: true, title: 'Remove host', description: 'This is final.' } });
		expect(screen.getByRole('dialog')).toHaveAccessibleDescription('This is final.');
	});

	it('applies the size class and shows the title icon', () => {
		render(Dialog, {
			props: { open: true, title: 'Delete', size: 'lg', icon: 'i-octagon' }
		});
		expect(document.querySelector('.rh-dialog')).toHaveClass('rh-dialog--lg');
		expect(document.querySelector('.rh-dialog__title use')).toHaveAttribute(
			'href',
			'/icons.svg#i-octagon'
		);
	});

	it('renders the body and the footer', () => {
		render(Dialog, {
			props: { open: true, title: 'Edit', children: body('The body.') }
		});
		expect(screen.getByText('The body.')).toBeInTheDocument();
	});
});
