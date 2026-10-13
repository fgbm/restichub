import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import ConfirmDialog from './ConfirmDialog.svelte';

describe('ConfirmDialog', () => {
	it('calls the confirm handler', async () => {
		const user = userEvent.setup();
		const onConfirm = vi.fn();
		render(ConfirmDialog, {
			props: {
				open: true,
				title: 'Delete user',
				message: 'This cannot be undone.',
				confirmLabel: 'Delete',
				onConfirm
			}
		});

		expect(screen.getByRole('dialog')).toHaveAccessibleName('Delete user');
		await user.click(screen.getByRole('button', { name: 'Delete' }));
		expect(onConfirm).toHaveBeenCalledOnce();
	});

	it('closes on cancel', async () => {
		const user = userEvent.setup();
		render(ConfirmDialog, {
			props: {
				open: true,
				title: 'Delete user',
				message: 'This cannot be undone.',
				confirmLabel: 'Delete'
			}
		});

		await user.click(screen.getByRole('button', { name: 'Cancel' }));
		await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
	});
});
