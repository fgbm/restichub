/*
 * The Users screen lists the active users first and changes the role of one (FR-TN-02).
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Users from './+page.svelte';

const mocks = vi.hoisted(() => ({
	listUsers: vi.fn(),
	updateUser: vi.fn(),
	pushToast: vi.fn()
}));

const route = vi.hoisted(() => ({ url: new URL('http://localhost/settings/access/users') }));

vi.mock('$app/state', () => ({ page: route }));
vi.mock('$app/navigation', () => ({
	goto: (address: string) => {
		route.url = new URL(address, 'http://localhost');
	}
}));
vi.mock('#lib/stores/session.svelte.ts', () => ({
	getSession: () => ({
		user: { id: 'u_01', username: 'admin', display_name: 'Anna', role: 'admin' },
		tenant: { id: 't_01', name: 'acme' },
		role: 'admin'
	})
}));
vi.mock('#lib/stores/toasts.svelte.ts', () => ({ pushToast: mocks.pushToast }));
vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		service: { ...actual.service, listUsers: mocks.listUsers, updateUser: mocks.updateUser }
	};
});

const users = [
	{
		id: 'u_01',
		username: 'apetrova',
		display_name: 'Anna Petrova',
		role: 'admin',
		disabled: false,
		shared: false
	},
	{
		id: 'u_02',
		username: 'duty',
		display_name: 'Duty shift',
		role: 'operator',
		disabled: false,
		shared: true
	},
	{
		id: 'u_03',
		username: 'okotova',
		display_name: 'Olga Kotova',
		role: 'viewer',
		disabled: true,
		shared: false
	}
];

beforeEach(() => {
	route.url = new URL('http://localhost/settings/access/users');
	for (const mock of Object.values(mocks)) mock.mockReset();
	mocks.listUsers.mockResolvedValue(users);
	mocks.updateUser.mockResolvedValue(users[1]);
});

describe('Users screen', () => {
	it('shows the active users with the login, the role and a shared account', async () => {
		render(Users);
		await screen.findByText('Anna Petrova');
		expect(screen.getByRole('button', { name: 'Active 2' })).toHaveAttribute(
			'aria-pressed',
			'true'
		);
		expect(screen.getByRole('button', { name: 'Disabled 1' })).toBeInTheDocument();
		expect(screen.queryByText('Olga Kotova')).toBeNull();
		expect(screen.getByText('apetrova')).toBeInTheDocument();
		expect(screen.getByText('shared account')).toBeInTheDocument();
		expect(document.querySelector('.status')).toBeNull();
	});

	it('changes the role of a user in a small dialog', async () => {
		const user = userEvent.setup();
		render(Users);
		await screen.findByText('Duty shift');
		await user.click(screen.getByRole('button', { name: 'More actions for Duty shift' }));
		await user.click(await screen.findByText('Change role'));
		const dialog = await screen.findByRole('dialog', { name: 'Change the role' });
		await user.selectOptions(within(dialog).getByLabelText('Role'), 'viewer');
		await user.click(within(dialog).getByRole('button', { name: 'Save' }));
		await waitFor(() => expect(mocks.updateUser).toHaveBeenCalledWith('u_02', { role: 'viewer' }));
	});
});
