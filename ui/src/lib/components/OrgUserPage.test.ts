/*
 * The organization user page shows one member with its role and the member actions (ADR-0025,
 * ADR-0029). The test replaces the session, the toast store and the service.
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '#lib/api/index.ts';
import OrgUserPage from './OrgUserPage.svelte';

const mocks = vi.hoisted(() => ({
	getUser: vi.fn(),
	updateUser: vi.fn(),
	resetUserPassword: vi.fn(),
	pushToast: vi.fn()
}));

vi.mock('#lib/stores/session.svelte.ts', () => ({
	getSession: () => ({
		user: { id: 'u_01', username: 'admin', display_name: 'Anna', role: 'admin' },
		tenant: { id: 't_01', name: 'acme' },
		role: 'admin',
		platform_admin: false
	})
}));
vi.mock('#lib/stores/toasts.svelte.ts', () => ({ pushToast: mocks.pushToast }));
vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		assignableRoles: () => ['admin', 'operator', 'viewer'],
		service: {
			...actual.service,
			getUser: mocks.getUser,
			updateUser: mocks.updateUser,
			resetUserPassword: mocks.resetUserPassword
		}
	};
});

function member(overrides: Record<string, unknown> = {}) {
	return {
		id: 'u_02',
		username: 'egorkin',
		display_name: 'Ivan Egorkin',
		role: 'operator',
		disabled: false,
		shared: false,
		platform_admin: false,
		...overrides
	};
}

beforeEach(() => {
	for (const mock of Object.values(mocks)) mock.mockReset();
	mocks.getUser.mockResolvedValue(member());
	mocks.updateUser.mockImplementation(async (_id: string, patch: Record<string, unknown>) =>
		member(patch)
	);
});

describe('Organization user page', () => {
	it('shows the role with its description and the account facts', async () => {
		render(OrgUserPage, { userId: 'u_02' });
		expect(await screen.findByRole('heading', { level: 1, name: 'Ivan Egorkin' })).toBeVisible();
		expect(mocks.getUser).toHaveBeenCalledWith('u_02');
		expect(screen.getByLabelText('Role')).toHaveValue('operator');
		expect(
			screen.getByText('Profiles, plans, manual runs, cancellation and restore.')
		).toBeVisible();
		expect(screen.getByText('egorkin')).toBeInTheDocument();
		expect(screen.queryByText('Shared account')).toBeNull();
	});

	it('saves the role at once and returns it on an error', async () => {
		const user = userEvent.setup();
		mocks.updateUser.mockRejectedValueOnce(
			new ApiError({ status: 409, code: 'conflict', title: 'Conflict' })
		);
		render(OrgUserPage, { userId: 'u_02' });
		const select = await screen.findByLabelText('Role');
		await user.selectOptions(select, 'viewer');
		await waitFor(() => expect(mocks.updateUser).toHaveBeenCalledWith('u_02', { role: 'viewer' }));
		await waitFor(() => expect(select).toHaveValue('operator'));
		expect(mocks.pushToast).toHaveBeenCalledWith('danger', expect.any(String));

		await user.selectOptions(select, 'admin');
		await waitFor(() =>
			expect(mocks.pushToast).toHaveBeenCalledWith('success', 'The role is changed.')
		);
		expect(select).toHaveValue('admin');
		expect(
			screen.getByText(
				'All organization operations, including users, agent revocation and installation.'
			)
		).toBeVisible();
	});

	it('asks before it disables the member', async () => {
		const user = userEvent.setup();
		render(OrgUserPage, { userId: 'u_02' });
		await user.click(await screen.findByRole('button', { name: 'More actions for Ivan Egorkin' }));
		await user.click(await screen.findByText('Disable user'));
		const dialog = await screen.findByRole('dialog', { name: 'Disable the user?' });
		expect(dialog).toHaveTextContent('Ivan Egorkin cannot work in the organization acme.');
		await user.click(within(dialog).getByRole('button', { name: 'Disable user' }));
		await waitFor(() => expect(mocks.updateUser).toHaveBeenCalledWith('u_02', { disabled: true }));
	});

	it('disables the reset of a shared account', async () => {
		mocks.getUser.mockResolvedValue(member({ shared: true }));
		render(OrgUserPage, { userId: 'u_02' });
		const reset = await screen.findByRole('button', { name: 'Reset password' });
		expect(reset).toBeDisabled();
		expect(reset.getAttribute('title')).toMatch(/belongs to other organizations/);
		expect(screen.getByText('Shared account')).toBeInTheDocument();
		expect(screen.getByLabelText('Role')).toBeEnabled();
	});

	it('disables every change of an account with the Platform Admin right', async () => {
		const user = userEvent.setup();
		mocks.getUser.mockResolvedValue(member({ platform_admin: true }));
		render(OrgUserPage, { userId: 'u_02' });
		const reset = await screen.findByRole('button', { name: 'Reset password' });
		expect(reset).toBeDisabled();
		expect(reset).toHaveAttribute('title', 'A Platform Admin manages this account');
		expect(screen.getByLabelText('Role')).toBeDisabled();
		await user.click(screen.getByRole('button', { name: 'More actions for Ivan Egorkin' }));
		const item = (await screen.findByText('Disable user')).closest('[data-disabled]');
		expect(item).toHaveTextContent('A Platform Admin manages this account');
	});

	it('disables the own role, the own disable switch and the own reset', async () => {
		mocks.getUser.mockResolvedValue(member({ id: 'u_01', role: 'admin' }));
		render(OrgUserPage, { userId: 'u_01' });
		expect(await screen.findByLabelText('Role')).toBeDisabled();
		const reset = screen.getByRole('button', { name: 'Reset password' });
		expect(reset).toBeDisabled();
		expect(reset).toHaveAttribute('title', 'This is your account');
	});
});
