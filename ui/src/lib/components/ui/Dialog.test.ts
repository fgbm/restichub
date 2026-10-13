import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { createRawSnippet, type Snippet } from 'svelte';
import { describe, expect, it, vi } from 'vitest';
import Dialog from './Dialog.svelte';

function markup(html: string): Snippet {
	return createRawSnippet(() => ({ render: () => html }));
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

	it('closes on the close button with an icon', async () => {
		const user = userEvent.setup();
		render(Dialog, { props: { open: true, title: 'Remove host' } });

		const close = screen.getByRole('button', { name: 'Close' });
		expect(close.querySelector('svg[data-icon="x"]')).toHaveAttribute('aria-hidden', 'true');
		await user.click(close);
		await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
	});

	it('is centered at the medium width by default and takes a size', () => {
		const { unmount } = render(Dialog, { props: { open: true, title: 'Create a user' } });
		expect(screen.getByRole('dialog')).toHaveClass('rh-dialog', 'rh-dialog--md');
		unmount();

		render(Dialog, { props: { open: true, title: 'Copy the token value', size: 'lg' } });
		expect(screen.getByRole('dialog')).toHaveClass('rh-dialog--lg');
	});

	it('renders the header, the body and the footer in order', () => {
		render(Dialog, {
			props: {
				open: true,
				title: 'Cancel the job?',
				description: 'The job stops after the current step.',
				footer: markup('<button type="button">Cancel job</button>')
			}
		});
		const dialog = screen.getByRole('dialog');
		const parts = [...dialog.children].map((child) => child.className);
		expect(parts).toEqual([
			expect.stringContaining('rh-dialog__head'),
			expect.stringContaining('rh-dialog__body'),
			expect.stringContaining('rh-dialog__foot')
		]);
		expect(dialog).toHaveAccessibleDescription('The job stops after the current step.');
	});

	it('hides the close button when it is not closable', () => {
		render(Dialog, { props: { open: true, title: 'Cancel the job?', closable: false } });
		expect(screen.queryByRole('button', { name: 'Close' })).not.toBeInTheDocument();
	});

	it('shows the title icon', () => {
		render(Dialog, { props: { open: true, title: 'Delete the repository?', icon: 'octagon' } });
		const heading = screen.getByRole('heading', { name: 'Delete the repository?' });
		expect(heading.querySelector('svg[data-icon="octagon"]')).not.toBeNull();
	});

	it('submits the form on Enter', async () => {
		const user = userEvent.setup();
		const onsubmit = vi.fn();
		render(Dialog, {
			props: {
				open: true,
				title: 'Create a user',
				onsubmit,
				children: markup('<label>User name <input name="username" /></label>'),
				footer: markup('<button type="submit">Create user</button>')
			}
		});

		await user.type(screen.getByLabelText('User name'), 'operator{Enter}');
		expect(onsubmit).toHaveBeenCalledOnce();
	});

	it('moves the focus into the dialog', async () => {
		render(Dialog, {
			props: {
				open: true,
				title: 'Create a user',
				children: markup('<label>User name <input name="username" /></label>')
			}
		});
		await waitFor(() =>
			expect(screen.getByRole('dialog')).toContainElement(document.activeElement as HTMLElement)
		);
	});
});
