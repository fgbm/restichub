import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import DialogHarness from '#lib/test/DialogHarness.svelte';

describe('Dialog', () => {
	it('renders the dialog with the correct aria attributes', () => {
		render(DialogHarness, { props: { open: true } });
		const dialog = screen.getByRole('dialog');
		expect(dialog).toHaveAttribute('aria-modal', 'true');
		expect(dialog).toHaveAccessibleName('Test dialog');
	});

	it('moves the focus into the dialog and traps Tab', async () => {
		const user = userEvent.setup();
		render(DialogHarness, { props: { open: true } });

		const close = screen.getByRole('button', { name: 'Close' });
		await waitFor(() => expect(close).toHaveFocus());

		const second = screen.getByRole('button', { name: 'Second action' });
		second.focus();
		await user.tab();
		expect(close).toHaveFocus();
	});

	it('closes on the Escape key', async () => {
		const user = userEvent.setup();
		render(DialogHarness, { props: { open: true } });

		expect(screen.getByRole('dialog')).toBeInTheDocument();
		await user.keyboard('{Escape}');
		await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
	});

	it('closes on the close button', async () => {
		const user = userEvent.setup();
		render(DialogHarness, { props: { open: true } });

		await user.click(screen.getByRole('button', { name: 'Close' }));
		await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
	});
});
