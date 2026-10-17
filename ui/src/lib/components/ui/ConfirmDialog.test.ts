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

	it('disables the confirm button and shows the reason when blocked', async () => {
		const onConfirm = vi.fn();
		render(ConfirmDialog, {
			props: {
				open: true,
				title: 'Delete repository',
				message: 'This cannot be undone.',
				confirmLabel: 'Delete',
				blocked: {
					code: 'repository_in_use',
					reason: 'Profiles use this repository.',
					links: [{ label: 'Alpha', kind: 'Profile', href: '/profiles/p1' }],
					more: 3
				},
				onConfirm
			}
		});

		expect(screen.getByRole('alert')).toHaveTextContent('Profiles use this repository.');
		expect(screen.getByRole('link', { name: 'Alpha' })).toHaveAttribute('href', '/profiles/p1');
		expect(screen.getByText('and 3 more')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Delete' })).toBeDisabled();
	});

	it('disables the confirm button while the check runs', () => {
		render(ConfirmDialog, {
			props: {
				open: true,
				title: 'Delete repository',
				message: 'This cannot be undone.',
				confirmLabel: 'Delete',
				checking: true
			}
		});

		expect(screen.getByRole('button', { name: 'Delete' })).toBeDisabled();
	});
});
