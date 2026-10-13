import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import Dialog from './Dialog.svelte';

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
});
