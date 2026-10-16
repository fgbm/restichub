/*
 * The platform Users screen lists every installation account and links each one to its record
 * page (ADR-0028, ADR-0029).
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import PlatformUsers from './+page.svelte';

const mocks = vi.hoisted(() => ({
	listPlatformUsers: vi.fn(),
	listTenants: vi.fn(),
	updatePlatformUser: vi.fn(),
	pushToast: vi.fn()
}));

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
		platform_admin: true
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
			updatePlatformUser: mocks.updatePlatformUser
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
	for (const mock of Object.values(mocks)) mock.mockReset();
	mocks.listPlatformUsers.mockResolvedValue([
		account('self', 'Self', true),
		account('u_02', 'Ivan Egorkin', true)
	]);
	mocks.listTenants.mockResolvedValue([]);
	mocks.updatePlatformUser.mockResolvedValue(undefined);
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
});
