/*
 * The organization user page shows one member with its role and the member actions (ADR-0025,
 * ADR-0029). A role below Admin gets the account facts only (AC-AU-02). The test replaces the
 * session, the toast store and the service.
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

const session = vi.hoisted(() => ({ role: 'admin' }));

vi.mock('#lib/stores/session.svelte.ts', () => ({
	getSession: () => ({
		user: { id: 'u_01', username: 'admin', display_name: 'Anna', role: session.role },
		tenant: { id: 't_01', name: 'acme' },
		role: session.role,
		platform_admin: false
	}),
	hasRole: (...roles: string[]) => roles.includes(session.role)
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
	session.role = 'admin';
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
		expect(screen.queryByText('Last sign-in')).toBeNull();
	});

	it('shows the last sign-in of the member', async () => {
		const signedIn = new Date(Date.now() - 2 * 3_600_000).toISOString();
		mocks.getUser.mockResolvedValue(member({ last_sign_in_at: signedIn }));
		render(OrgUserPage, { userId: 'u_02' });
		const time = await screen.findByText('2 hours ago', { selector: 'time' });
		expect(time).toHaveAttribute('datetime', signedIn);
		expect(screen.getByText('Last sign-in')).toBeInTheDocument();
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

	it('asks before it disables the member in the dangerous section', async () => {
		const user = userEvent.setup();
		render(OrgUserPage, { userId: 'u_02' });
		const danger = (await screen.findByRole('button', { name: /Dangerous actions/ })).closest(
			'section'
		) as HTMLElement;
		await user.click(within(danger).getByRole('button', { name: 'Disable user' }));
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
		mocks.getUser.mockResolvedValue(member({ platform_admin: true }));
		render(OrgUserPage, { userId: 'u_02' });
		const reset = await screen.findByRole('button', { name: 'Reset password' });
		expect(reset).toBeDisabled();
		expect(reset).toHaveAttribute('title', 'A Platform Admin manages this account');
		expect(screen.getByLabelText('Role')).toBeDisabled();
		const danger = screen
			.getByRole('button', { name: /Dangerous actions/ })
			.closest('section') as HTMLElement;
		const disable = within(danger).getByRole('button', { name: 'Disable user' });
		expect(disable).toBeDisabled();
		expect(disable).toHaveAttribute('title', 'A Platform Admin manages this account');
	});

	it('disables the own role, the own disable switch and the own reset', async () => {
		mocks.getUser.mockResolvedValue(member({ id: 'u_01', role: 'admin' }));
		render(OrgUserPage, { userId: 'u_01' });
		expect(await screen.findByLabelText('Role')).toBeDisabled();
		const reset = screen.getByRole('button', { name: 'Reset password' });
		expect(reset).toBeDisabled();
		expect(reset).toHaveAttribute('title', 'This is your account');
	});

	it('keeps the state of the member in the dangerous section only', async () => {
		render(OrgUserPage, { userId: 'u_02' });
		await screen.findByRole('heading', { level: 1, name: 'Ivan Egorkin' });
		// The sentence of the organization user list is the subtitle of the record.
		expect(
			screen.getByText('Members of the organization acme and their roles.')
		).toBeInTheDocument();
		const header = document.querySelector<HTMLElement>('.page-header__actions') as HTMLElement;
		expect(within(header).getByRole('button', { name: 'Reset password' })).toBeInTheDocument();
		expect(within(header).queryByRole('button', { name: /More actions/ })).toBeNull();

		const danger = screen
			.getByRole('button', { name: /Dangerous actions/ })
			.closest('section') as HTMLElement;
		expect(
			within(danger).getByText(/The person cannot work in the organization/)
		).toBeInTheDocument();
		expect(within(danger).getByRole('button', { name: 'Disable user' })).toHaveClass('btn--danger');
	});

	it('offers the enable action of a disabled member in the dangerous section', async () => {
		mocks.getUser.mockResolvedValue(member({ disabled: true }));
		render(OrgUserPage, { userId: 'u_02' });
		await screen.findByRole('heading', { level: 1, name: 'Ivan Egorkin' });
		const danger = screen
			.getByRole('button', { name: /Dangerous actions/ })
			.closest('section') as HTMLElement;
		expect(within(danger).getByText(/can sign in to the organization again/)).toBeInTheDocument();
		expect(within(danger).getByRole('button', { name: 'Enable user' })).toHaveClass(
			'btn--secondary'
		);
	});

	// The UI hides what the API refuses: an Admin changes a member (ADR-0023, AC-AU-02).
	it.each(['operator', 'viewer'] as const)(
		'ac_au_02_shows_the_member_to_a_%s_without_a_way_to_change_it',
		async (role) => {
			session.role = role;
			render(OrgUserPage, { userId: 'u_02' });
			expect(await screen.findByRole('heading', { level: 1, name: 'Ivan Egorkin' })).toBeVisible();
			expect(screen.getByLabelText('Role')).toBeDisabled();
			expect(screen.getByLabelText('Role')).toHaveValue('operator');
			expect(screen.queryByRole('button', { name: 'Reset password' })).toBeNull();
			expect(document.querySelector('[data-section="danger"]')).toBeNull();
		}
	);
});
