/*
 * The platform Users screen lists every installation account and links each one to its record
 * page (ADR-0028, ADR-0029). Without the Platform Admin right the screen has no account action
 * (AC-AU-02).
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import PlatformUsers from './+page.svelte';

const mocks = vi.hoisted(() => ({
	listPlatformUsers: vi.fn(),
	listTenants: vi.fn(),
	updatePlatformUser: vi.fn(),
	createPlatformUser: vi.fn(),
	pushToast: vi.fn()
}));

const session = vi.hoisted(() => ({ platform_admin: true }));

const route = vi.hoisted(() => ({ url: new URL('http://localhost/platform/users') }));

vi.mock('$app/state', () => ({ page: route }));
vi.mock('$app/paths', () => ({
	resolve: (path: string, params?: Record<string, string>) =>
		(params ? path.replace('[id]', params.id) : path).replace('/(app)', '')
}));
vi.mock('$app/navigation', () => ({
	goto: (address: string) => {
		route.url = new URL(address, 'http://localhost');
	}
}));
vi.mock('#lib/stores/session.svelte.ts', () => ({
	getSession: () => ({
		user: { id: 'self', username: 'neimp', display_name: 'Self', role: 'admin' },
		tenant: null,
		role: 'admin',
		platform_admin: session.platform_admin
	})
}));
vi.mock('#lib/stores/toasts.svelte.ts', () => ({ pushToast: mocks.pushToast }));
vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		service: {
			...actual.service,
			listPlatformUsers: mocks.listPlatformUsers,
			listTenants: mocks.listTenants,
			updatePlatformUser: mocks.updatePlatformUser,
			createPlatformUser: mocks.createPlatformUser
		}
	};
});

function account(id: string, displayName: string, platformAdmin: boolean) {
	return {
		id,
		username: id,
		display_name: displayName,
		email: null,
		email_verified: false,
		disabled: false,
		platform_admin: platformAdmin,
		memberships: []
	};
}

beforeEach(() => {
	route.url = new URL('http://localhost/platform/users');
	session.platform_admin = true;
	for (const mock of Object.values(mocks)) mock.mockReset();
	mocks.listPlatformUsers.mockResolvedValue([
		account('self', 'Self', true),
		account('u_02', 'Ivan Egorkin', true)
	]);
	mocks.listTenants.mockResolvedValue([]);
	mocks.updatePlatformUser.mockResolvedValue(undefined);
	mocks.createPlatformUser.mockResolvedValue(undefined);
});

describe('Platform Users screen', () => {
	it('links each account to its record page', async () => {
		render(PlatformUsers);
		const link = await screen.findByRole('link', { name: 'Ivan Egorkin' });
		expect(link).toHaveAttribute('href', '/platform/users/u_02');
	});

	it('offers the reset, the right and the disable switch but not the memberships', async () => {
		const user = userEvent.setup();
		render(PlatformUsers);
		await user.click(await screen.findByRole('button', { name: 'More actions for Ivan Egorkin' }));
		expect(await screen.findByText('Reset password')).toBeInTheDocument();
		expect(screen.getByText('Remove the Platform Admin right')).toBeInTheDocument();
		expect(screen.queryByText('Change memberships')).toBeNull();
	});

	it('asks before it removes the right of another account', async () => {
		const user = userEvent.setup();
		render(PlatformUsers);
		await user.click(await screen.findByRole('button', { name: 'More actions for Ivan Egorkin' }));
		await user.click(await screen.findByText('Remove the Platform Admin right'));
		const dialog = await screen.findByRole('dialog', { name: 'Remove the Platform Admin right?' });
		expect(mocks.updatePlatformUser).not.toHaveBeenCalled();
		await user.click(within(dialog).getByRole('button', { name: 'Remove the right' }));
		await waitFor(() =>
			expect(mocks.updatePlatformUser).toHaveBeenCalledWith('u_02', { platform_admin: false })
		);
	});

	it('disables the own account actions with the reason', async () => {
		const user = userEvent.setup();
		render(PlatformUsers);
		await user.click(await screen.findByRole('button', { name: 'More actions for Self' }));
		const reset = (await screen.findByText('Reset password')).closest('[data-disabled]');
		expect(reset).toHaveTextContent('This is your account');
	});

	it('keeps the create button enabled and shows the password rule on submit', async () => {
		const user = userEvent.setup();
		render(PlatformUsers);
		await user.click(await screen.findByRole('button', { name: 'Create user' }));
		const dialog = await screen.findByRole('dialog', { name: 'Create an installation account' });
		const password = within(dialog).getByLabelText(/Initial password/);
		const submit = within(dialog).getByRole('button', { name: 'Create user' });

		await user.type(within(dialog).getByLabelText(/User name/), 'ivan');
		await user.type(within(dialog).getByLabelText(/Display name/), 'Ivan');
		await user.type(password, 'abcdef');
		expect(within(dialog).getByText('6 of 12 characters')).toHaveClass('rh-field__help');
		expect(submit).toBeEnabled();

		await user.click(submit);
		expect(mocks.createPlatformUser).not.toHaveBeenCalled();
		expect(password).toHaveFocus();
		expect(password).toHaveAttribute('aria-invalid', 'true');
		expect(within(dialog).getByText('6 of 12 characters')).toHaveClass('rh-field__error');

		await user.type(password, 'ghijkl');
		expect(within(dialog).getByText('12 characters — long enough')).toBeVisible();
		expect(password).not.toHaveAttribute('aria-invalid');
		await user.click(submit);
		await waitFor(() =>
			expect(mocks.createPlatformUser).toHaveBeenCalledWith(
				expect.objectContaining({ username: 'ivan', password: 'abcdefghijkl' })
			)
		);
	});

	it('names the empty required fields on submit', async () => {
		const user = userEvent.setup();
		render(PlatformUsers);
		await user.click(await screen.findByRole('button', { name: 'Create user' }));
		const dialog = await screen.findByRole('dialog', { name: 'Create an installation account' });
		await user.click(within(dialog).getByRole('button', { name: 'Create user' }));
		expect(mocks.createPlatformUser).not.toHaveBeenCalled();
		expect(within(dialog).getAllByText('This field is required.')).toHaveLength(2);
		expect(within(dialog).getByLabelText(/User name/)).toHaveFocus();
		expect(within(dialog).getByText('At least 12 characters.')).toHaveClass('rh-field__error');
	});

	// The UI hides what the API refuses: a platform operation needs the installation-wide right
	// (ADR-0029, AC-AU-02).
	it('ac_au_02_hides_every_account_action_without_the_platform_admin_right', async () => {
		session.platform_admin = false;
		render(PlatformUsers);

		expect(await screen.findByRole('link', { name: 'Ivan Egorkin' })).toHaveAttribute(
			'href',
			'/platform/users/u_02'
		);
		expect(screen.queryByRole('button', { name: 'Create user' })).toBeNull();
		expect(screen.queryByRole('button', { name: 'More actions for Ivan Egorkin' })).toBeNull();
	});
});
