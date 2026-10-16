/*
 * The platform user page shows one installation account with its memberships and the account
 * actions (ADR-0028, ADR-0029). The test replaces the session, the toast store and the service, so
 * it renders the page alone.
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '#lib/api/index.ts';
import PlatformUserPage from './PlatformUserPage.svelte';

const mocks = vi.hoisted(() => ({
	getPlatformUser: vi.fn(),
	listTenants: vi.fn(),
	listPlatformUsers: vi.fn(),
	setPlatformMembershipRole: vi.fn(),
	addPlatformMembership: vi.fn(),
	removePlatformMembership: vi.fn(),
	updatePlatformUser: vi.fn(),
	resetPlatformUserPassword: vi.fn(),
	pushToast: vi.fn()
}));

vi.mock('#lib/stores/session.svelte.ts', () => ({
	getSession: () => ({
		user: { id: 'self', username: 'neimp', display_name: 'Self', role: 'admin' },
		tenant: { id: 't_01', name: 'acme' },
		role: 'admin',
		platform_admin: true
	})
}));
vi.mock('#lib/stores/toasts.svelte.ts', () => ({ pushToast: mocks.pushToast }));
vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	const { pushToast: _, ...service } = mocks;
	void _;
	return { ...actual, service: { ...actual.service, ...service } };
});

function tenant(id: string, name: string, status = 'active') {
	return { id, name, status, created_at: '2026-01-01T00:00:00Z', user_count: 1, admin_state: 'ok' };
}

function account(overrides: Record<string, unknown> = {}) {
	return {
		id: 'u_02',
		username: 'egorkin',
		display_name: 'Ivan Egorkin',
		email: null,
		email_verified: false,
		disabled: false,
		platform_admin: false,
		memberships: [
			{ tenant_id: 't_01', tenant_name: 'acme', role: 'operator', disabled: false },
			{ tenant_id: 't_03', tenant_name: 'initech', role: 'viewer', disabled: false }
		],
		platform_admin_count: 2,
		...overrides
	};
}

const self = account({
	id: 'self',
	username: 'neimp',
	display_name: 'Self',
	email: 'self@example.com',
	platform_admin: true,
	memberships: [{ tenant_id: 't_01', tenant_name: 'acme', role: 'admin', disabled: false }]
});

beforeEach(() => {
	for (const mock of Object.values(mocks)) mock.mockReset();
	mocks.getPlatformUser.mockResolvedValue(account());
	mocks.listTenants.mockResolvedValue([
		tenant('t_01', 'acme'),
		tenant('t_02', 'globex'),
		tenant('t_03', 'initech', 'suspended'),
		tenant('t_04', 'umbrella')
	]);
	mocks.setPlatformMembershipRole.mockResolvedValue(undefined);
	mocks.addPlatformMembership.mockResolvedValue(undefined);
	mocks.removePlatformMembership.mockResolvedValue(undefined);
	mocks.updatePlatformUser.mockResolvedValue(undefined);
	mocks.resetPlatformUserPassword.mockResolvedValue(undefined);
});

describe('Platform user page', () => {
	it('names the person in the heading and shows the account facts', async () => {
		render(PlatformUserPage, { userId: 'u_02' });
		expect(await screen.findByRole('heading', { level: 1, name: 'Ivan Egorkin' })).toBeVisible();
		expect(mocks.getPlatformUser).toHaveBeenCalledWith('u_02');
		const panel = screen.getByRole('heading', { name: 'Account' }).closest('section');
		expect(panel).not.toBeNull();
		const facts = within(panel as HTMLElement);
		expect(facts.getByText('egorkin')).toBeInTheDocument();
		expect(facts.getByText('Active')).toBeInTheDocument();
		expect(facts.getByText('Platform Admin')).toBeInTheDocument();
		expect(facts.queryByText('Email')).toBeNull();
		expect(facts.queryByText('Last sign-in')).toBeNull();
	});

	it('shows the last sign-in of the record', async () => {
		const signedIn = new Date(Date.now() - 2 * 3_600_000).toISOString();
		mocks.getPlatformUser.mockResolvedValue(account({ last_sign_in_at: signedIn }));
		render(PlatformUserPage, { userId: 'u_02' });
		const time = await screen.findByText('2 hours ago', { selector: 'time' });
		expect(time).toHaveAttribute('datetime', signedIn);
		expect(screen.getByText('Last sign-in')).toBeInTheDocument();
	});

	it('marks a membership of a suspended organization', async () => {
		render(PlatformUserPage, { userId: 'u_02' });
		await screen.findByText('initech');
		const row = screen.getByText('initech').closest('tr') as HTMLElement;
		expect(within(row).getByText('Suspended')).toBeInTheDocument();
	});

	it('offers only the active organizations without a membership in the add row', async () => {
		const user = userEvent.setup();
		render(PlatformUserPage, { userId: 'u_02' });
		const select = (await screen.findByLabelText('Organization')) as HTMLSelectElement;
		const names = [...select.options].filter((option) => !option.disabled).map((o) => o.text);
		expect(names).toEqual(['globex', 'umbrella']);
		const add = screen.getByRole('button', { name: 'Add an organization' });
		expect(add).toBeDisabled();
		expect(screen.getByLabelText('Role')).toHaveValue('viewer');

		await user.selectOptions(select, 't_02');
		await user.click(add);
		await waitFor(() =>
			expect(mocks.addPlatformMembership).toHaveBeenCalledWith('u_02', {
				tenant_id: 't_02',
				role: 'viewer'
			})
		);
		expect(mocks.pushToast).toHaveBeenCalledWith('success', 'The membership is added.');
	});

	it('saves a role at once and returns the select to the saved role on an error', async () => {
		const user = userEvent.setup();
		mocks.setPlatformMembershipRole.mockRejectedValueOnce(
			new ApiError({ status: 403, code: 'forbidden', title: 'Forbidden' })
		);
		render(PlatformUserPage, { userId: 'u_02' });
		const select = await screen.findByLabelText('Role in the organization acme');
		await user.selectOptions(select, 'admin');
		await waitFor(() =>
			expect(mocks.setPlatformMembershipRole).toHaveBeenCalledWith('u_02', 't_01', {
				role: 'admin'
			})
		);
		await waitFor(() => expect(select).toHaveValue('operator'));
		expect(mocks.pushToast).toHaveBeenCalledWith('danger', expect.any(String));

		await user.selectOptions(select, 'viewer');
		await waitFor(() =>
			expect(mocks.pushToast).toHaveBeenCalledWith('success', 'The role is changed.')
		);
		expect(select).toHaveValue('viewer');
	});

	it('asks before it removes a membership', async () => {
		const user = userEvent.setup();
		render(PlatformUserPage, { userId: 'u_02' });
		await user.click(
			await screen.findByRole('button', { name: 'Remove from the organization acme' })
		);
		const dialog = await screen.findByRole('dialog', { name: 'Remove from the organization?' });
		expect(dialog).toHaveTextContent('Ivan Egorkin loses the access to the organization acme.');
		expect(mocks.removePlatformMembership).not.toHaveBeenCalled();
		await user.click(within(dialog).getByRole('button', { name: 'Remove' }));
		await waitFor(() =>
			expect(mocks.removePlatformMembership).toHaveBeenCalledWith('u_02', 't_01')
		);
	});

	it('asks before it disables the account', async () => {
		const user = userEvent.setup();
		render(PlatformUserPage, { userId: 'u_02' });
		await user.click(await screen.findByRole('button', { name: 'More actions for Ivan Egorkin' }));
		await user.click(await screen.findByText('Disable user'));
		const dialog = await screen.findByRole('dialog', { name: 'Disable the user?' });
		await user.click(within(dialog).getByRole('button', { name: 'Disable user' }));
		await waitFor(() =>
			expect(mocks.updatePlatformUser).toHaveBeenCalledWith('u_02', { disabled: true })
		);
	});

	it('resets the password through the platform endpoint', async () => {
		const user = userEvent.setup();
		render(PlatformUserPage, { userId: 'u_02' });
		await user.click(await screen.findByRole('button', { name: 'Reset password' }));
		const dialog = await screen.findByRole('dialog', { name: 'Reset the password' });
		await user.type(within(dialog).getByLabelText(/Password/), 'a long enough password');
		await user.click(within(dialog).getByRole('button', { name: 'Reset password' }));
		await waitFor(() =>
			expect(mocks.resetPlatformUserPassword).toHaveBeenCalledWith('u_02', {
				new_password: 'a long enough password'
			})
		);
	});

	it('disables the changes of the own account with the reason', async () => {
		const user = userEvent.setup();
		mocks.getPlatformUser.mockResolvedValue(self);
		render(PlatformUserPage, { userId: 'self' });
		const reset = await screen.findByRole('button', { name: 'Reset password' });
		expect(reset).toBeDisabled();
		expect(reset).toHaveAttribute('title', 'This is your account');
		expect(
			screen.getByRole('button', { name: 'Remove from the organization acme' })
		).toBeDisabled();
		expect(screen.getByLabelText('Role in the organization acme')).toBeDisabled();
		expect(screen.getByText('self@example.com')).toBeInTheDocument();
		expect(screen.getByText('not verified')).toBeInTheDocument();

		await user.click(screen.getByRole('button', { name: 'More actions for Self' }));
		const revoke = (await screen.findByText('Remove the Platform Admin right')).closest(
			'[data-disabled]'
		);
		expect(revoke).toHaveTextContent('This is your account');
		expect(screen.getByText('Disable user').closest('[data-disabled]')).not.toBeNull();
	});

	it('locks the right of the last Platform Admin from the count of the record', async () => {
		const user = userEvent.setup();
		mocks.getPlatformUser.mockResolvedValue(
			account({ platform_admin: true, platform_admin_count: 1 })
		);
		render(PlatformUserPage, { userId: 'u_02' });
		await user.click(await screen.findByRole('button', { name: 'More actions for Ivan Egorkin' }));
		const revoke = (await screen.findByText('Remove the Platform Admin right')).closest(
			'[data-disabled]'
		);
		expect(revoke).toHaveTextContent('This is the last Platform Admin');
		expect(mocks.listPlatformUsers).not.toHaveBeenCalled();
	});

	it('explains the empty memberships of a Platform Admin', async () => {
		mocks.getPlatformUser.mockResolvedValue(account({ platform_admin: true, memberships: [] }));
		render(PlatformUserPage, { userId: 'u_02' });
		expect(await screen.findByText('The user is not a member of an organization.')).toBeVisible();
		expect(
			screen.getByText(
				'The Platform Admin right gives no access to the data of an organization. Add an organization to work in it.'
			)
		).toBeVisible();
	});
});
