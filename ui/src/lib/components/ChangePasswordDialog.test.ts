/*
 * The own password change of the user menu. The test replaces the toast store and the service.
 */
import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '#lib/api/index.ts';
import ChangePasswordDialog from './ChangePasswordDialog.svelte';

const mocks = vi.hoisted(() => ({
	changePassword: vi.fn(),
	pushToast: vi.fn()
}));

vi.mock('#lib/stores/toasts.svelte.ts', () => ({ pushToast: mocks.pushToast }));
vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		service: { ...actual.service, changePassword: mocks.changePassword }
	};
});

beforeEach(() => {
	for (const mock of Object.values(mocks)) mock.mockReset();
});

async function fill(current: string, next: string): Promise<ReturnType<typeof userEvent.setup>> {
	const user = userEvent.setup();
	if (current) await user.type(screen.getByLabelText(/Current password/), current);
	if (next) await user.type(screen.getByLabelText(/New password/), next);
	await user.click(screen.getByRole('button', { name: 'Change password' }));
	return user;
}

describe('Change password dialog', () => {
	it('says that the other sessions end', () => {
		render(ChangePasswordDialog, { open: true });
		expect(screen.getByText('Your other sessions stop. This session continues.')).toBeVisible();
	});

	it('sends the passwords, shows a toast and closes', async () => {
		mocks.changePassword.mockResolvedValue(undefined);
		render(ChangePasswordDialog, { open: true });
		await fill('old-secret', 'a-new-long-password');
		await waitFor(() =>
			expect(mocks.changePassword).toHaveBeenCalledWith({
				current_password: 'old-secret',
				new_password: 'a-new-long-password'
			})
		);
		expect(mocks.pushToast).toHaveBeenCalledWith('success', 'The password is changed.');
		await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
	});

	it('shows a wrong current password on its field', async () => {
		mocks.changePassword.mockRejectedValue(
			new ApiError({
				status: 400,
				code: 'invalid_argument',
				title: 'Bad Request',
				field: 'current_password',
				reason: 'invalid_credentials'
			})
		);
		render(ChangePasswordDialog, { open: true });
		await fill('wrong', 'a-new-long-password');
		const current = screen.getByLabelText(/Current password/);
		await waitFor(() => expect(current).toHaveAttribute('aria-invalid', 'true'));
		expect(screen.getByText('The current password is not correct.')).toBeVisible();
		expect(current).toHaveFocus();
		expect(screen.getByRole('dialog')).toBeVisible();
		expect(mocks.pushToast).not.toHaveBeenCalled();
	});

	it('does not send the form when a field is not correct', async () => {
		render(ChangePasswordDialog, { open: true });
		await fill('', 'short');
		expect(mocks.changePassword).not.toHaveBeenCalled();
		expect(screen.getByLabelText(/Current password/)).toHaveFocus();
		expect(screen.getByLabelText(/New password/)).toHaveAttribute('aria-invalid', 'true');
	});

	it('shows an error without a field above the form', async () => {
		mocks.changePassword.mockRejectedValue(
			new ApiError({ status: 429, code: 'rate_limited', title: 'Too Many Requests' })
		);
		render(ChangePasswordDialog, { open: true });
		await fill('old-secret', 'a-new-long-password');
		expect(await screen.findByRole('alert')).toBeVisible();
		expect(screen.getByRole('dialog')).toBeVisible();
	});
});
