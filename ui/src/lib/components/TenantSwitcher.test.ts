/*
 * The tenant switcher (ADR-0025): the menu, the roles, the check mark, the filter, the keyboard,
 * the reload on open, the error toast, the click outside and the platform entry (ADR-0029).
 */
import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { UnauthorizedError } from '#lib/api/index.ts';
import { getSession, setSession } from '#lib/stores/session.svelte.ts';
import { getToasts } from '#lib/stores/toasts.svelte.ts';
import TenantSwitcher from './TenantSwitcher.svelte';

const mocks = vi.hoisted(() => ({
	listMemberships: vi.fn(),
	switchTenant: vi.fn(),
	invalidateAll: vi.fn(),
	goto: vi.fn()
}));

vi.mock('$app/navigation', () => ({
	goto: mocks.goto,
	invalidateAll: mocks.invalidateAll
}));
vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));

vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		service: {
			...actual.service,
			listMemberships: mocks.listMemberships,
			switchTenant: mocks.switchTenant
		}
	};
});

const tenants = [
	{ id: 't_01', name: 'acme', role: 'admin' as const },
	{ id: 't_02', name: 'globex', role: 'operator' as const }
];

function session(id: string, name: string, platformAdmin = false) {
	return {
		user: {
			id: 'u_01',
			username: 'alice',
			display_name: 'Alice',
			disabled: false,
			role: 'admin' as const,
			shared: false
		},
		tenant: { id, name },
		role: 'admin' as const,
		platform_admin: platformAdmin
	};
}

describe('the tenant switcher', () => {
	beforeEach(() => {
		mocks.listMemberships.mockReset();
		mocks.switchTenant.mockReset();
		mocks.invalidateAll.mockReset();
		mocks.goto.mockReset();
		mocks.listMemberships.mockResolvedValue(tenants);
		mocks.invalidateAll.mockResolvedValue(undefined);
		mocks.switchTenant.mockImplementation(async (id: string) =>
			session(id, tenants.find((tenant) => tenant.id === id)?.name ?? '')
		);
		setSession(session('t_01', 'acme'));
	});

	it('lists the memberships with the role and a check mark on the current tenant', async () => {
		const user = userEvent.setup();
		render(TenantSwitcher);

		await user.click(screen.getByRole('button', { name: /Switch organization/ }));
		await waitFor(() => expect(screen.getByRole('menu')).toBeInTheDocument());

		const current = screen.getByRole('menuitemradio', { name: /acme/ });
		expect(current).toHaveAttribute('aria-checked', 'true');
		expect(current.querySelector('svg[data-icon="check"]')).not.toBeNull();
		const other = screen.getByRole('menuitemradio', { name: /globex/ });
		expect(other).toHaveAttribute('aria-checked', 'false');
		expect(other.querySelector('svg[data-icon="check"]')).toBeNull();
	});

	it('ac_au_12_shows_the_empty_state_when_the_account_holds_no_membership', async () => {
		mocks.listMemberships.mockResolvedValue([]);
		const user = userEvent.setup();
		render(TenantSwitcher);

		await user.click(screen.getByRole('button', { name: /Switch organization/ }));
		await waitFor(() => expect(screen.getByText('No organizations')).toBeInTheDocument());
	});

	it('reloads the membership list on every open', async () => {
		const user = userEvent.setup();
		render(TenantSwitcher);
		const button = screen.getByRole('button', { name: /Switch organization/ });

		await user.click(button);
		await waitFor(() => expect(mocks.listMemberships).toHaveBeenCalledTimes(1));
		await user.keyboard('{Escape}');
		await user.click(button);
		await waitFor(() => expect(mocks.listMemberships).toHaveBeenCalledTimes(2));
	});

	it('redirects to the sign-in screen when the list call loses the session', async () => {
		mocks.listMemberships.mockRejectedValue(
			new UnauthorizedError({ status: 401, code: 'unauthenticated', title: 'Unauthorized' })
		);
		const user = userEvent.setup();
		render(TenantSwitcher);

		await user.click(screen.getByRole('button', { name: /Switch organization/ }));

		await waitFor(() => expect(mocks.goto).toHaveBeenCalledWith('/(auth)/sign-in'));
		expect(getSession()).toBeNull();
	});

	it('moves with the arrow keys and switches with Enter on the active item', async () => {
		const user = userEvent.setup();
		render(TenantSwitcher);

		await user.click(screen.getByRole('button', { name: /Switch organization/ }));
		await waitFor(() => expect(screen.getByRole('menu')).toBeInTheDocument());

		await user.keyboard('{ArrowDown}');
		await user.keyboard('{Enter}');
		await waitFor(() => expect(mocks.switchTenant).toHaveBeenCalledWith('t_02'));
		await waitFor(() => expect(mocks.invalidateAll).toHaveBeenCalled());
	});

	it('closes on Escape and returns the focus to the button', async () => {
		const user = userEvent.setup();
		render(TenantSwitcher);

		const button = screen.getByRole('button', { name: /Switch organization/ });
		await user.click(button);
		await waitFor(() => expect(screen.getByRole('menu')).toBeInTheDocument());

		await user.keyboard('{Escape}');
		await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument());
		expect(button).toHaveFocus();
	});

	it('closes on a click outside', async () => {
		const user = userEvent.setup();
		render(TenantSwitcher);

		await user.click(screen.getByRole('button', { name: /Switch organization/ }));
		await waitFor(() => expect(screen.getByRole('menu')).toBeInTheDocument());

		await user.click(document.body);
		await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument());
	});

	it('shows an error toast when the switch fails', async () => {
		mocks.switchTenant.mockRejectedValue(new Error('boom'));
		const user = userEvent.setup();
		render(TenantSwitcher);

		await user.click(screen.getByRole('button', { name: /Switch organization/ }));
		await waitFor(() => expect(screen.getByRole('menu')).toBeInTheDocument());
		await user.click(screen.getByRole('menuitemradio', { name: /globex/ }));

		await waitFor(() => expect(getToasts().some((toast) => toast.tone === 'danger')).toBe(true));
		expect(mocks.invalidateAll).not.toHaveBeenCalled();
	});

	it('filters the list when the caller holds more than seven memberships', async () => {
		mocks.listMemberships.mockResolvedValue([
			...tenants,
			{ id: 't_03', name: 'initech', role: 'viewer' as const },
			{ id: 't_04', name: 'northwind', role: 'viewer' as const },
			{ id: 't_05', name: 'umbrella', role: 'viewer' as const },
			{ id: 't_06', name: 'brightline', role: 'viewer' as const },
			{ id: 't_07', name: 'redwood', role: 'viewer' as const },
			{ id: 't_08', name: 'summit', role: 'viewer' as const }
		]);
		const user = userEvent.setup();
		render(TenantSwitcher);

		await user.click(screen.getByRole('button', { name: /Switch organization/ }));
		const filter = await screen.findByRole('searchbox', { name: 'Filter organizations' });
		await user.type(filter, 'glob');
		await waitFor(() => expect(screen.queryByRole('menuitemradio', { name: /acme/ })).toBeNull());
		expect(screen.getByRole('menuitemradio', { name: /globex/ })).toBeInTheDocument();
	});

	it('ac_au_13_lists_real_organizations_only', async () => {
		const user = userEvent.setup();
		render(TenantSwitcher);

		await user.click(screen.getByRole('button', { name: /Switch organization/ }));
		await waitFor(() =>
			expect(screen.getByRole('menuitemradio', { name: /acme/ })).toBeInTheDocument()
		);
		// The internal platform tenant is no entry (ADR-0029).
		expect(screen.queryByRole('menuitemradio', { name: /Platform/ })).toBeNull();
	});

	it('shows no platform entry without the Platform Admin right', async () => {
		const user = userEvent.setup();
		render(TenantSwitcher);

		await user.click(screen.getByRole('button', { name: /Switch organization/ }));
		await waitFor(() => expect(screen.getByRole('menu')).toBeInTheDocument());
		expect(screen.queryByRole('menuitem', { name: 'Platform administration' })).toBeNull();
		expect(screen.queryByRole('separator')).toBeNull();
	});

	it('shows the platform entry after a divider and opens the platform screens', async () => {
		setSession(session('t_01', 'acme', true));
		const user = userEvent.setup();
		render(TenantSwitcher);

		await user.click(screen.getByRole('button', { name: /Switch organization/ }));
		const entry = await screen.findByRole('menuitem', { name: 'Platform administration' });
		expect(screen.getByRole('separator')).toBeInTheDocument();
		expect(entry).not.toHaveAttribute('aria-checked');
		expect(entry.querySelector('svg[data-icon="shield"]')).not.toBeNull();
		expect(entry.querySelector('svg[data-icon="check"]')).toBeNull();
		expect(entry).toHaveTextContent('Platform administration');

		await user.click(entry);
		await waitFor(() => expect(mocks.goto).toHaveBeenCalledWith('/(app)/platform/tenants'));
		expect(mocks.switchTenant).not.toHaveBeenCalled();
		expect(mocks.invalidateAll).not.toHaveBeenCalled();
		expect(screen.queryByRole('menu')).not.toBeInTheDocument();
	});

	it('reaches the platform entry with the arrow keys and opens it with Enter', async () => {
		setSession(session('t_01', 'acme', true));
		const user = userEvent.setup();
		render(TenantSwitcher);

		await user.click(screen.getByRole('button', { name: /Switch organization/ }));
		const menu = await screen.findByRole('menu');
		await user.keyboard('{ArrowDown}{ArrowDown}');
		expect(menu).toHaveAttribute('aria-activedescendant', 'tenant-switch-platform');
		await user.keyboard('{Enter}');

		await waitFor(() => expect(mocks.goto).toHaveBeenCalledWith('/(app)/platform/tenants'));
		expect(mocks.switchTenant).not.toHaveBeenCalled();
	});

	it('keeps the platform entry when the filter matches no organization', async () => {
		setSession(session('t_01', 'acme', true));
		mocks.listMemberships.mockResolvedValue(
			Array.from({ length: 8 }, (_, index) => ({
				id: `t_0${index + 1}`,
				name: `org${index + 1}`,
				role: 'viewer' as const
			}))
		);
		const user = userEvent.setup();
		render(TenantSwitcher);

		await user.click(screen.getByRole('button', { name: /Switch organization/ }));
		const filter = await screen.findByRole('searchbox', { name: 'Filter organizations' });
		await user.type(filter, 'zzz');
		await waitFor(() => expect(screen.queryAllByRole('menuitemradio')).toHaveLength(0));
		expect(screen.getByRole('menuitem', { name: 'Platform administration' })).toBeInTheDocument();
		expect(screen.getByRole('menu')).toHaveAttribute(
			'aria-activedescendant',
			'tenant-switch-platform'
		);
	});

	it('shows the empty state and the platform entry for an admin with no membership', async () => {
		setSession({ ...session('t_01', 'acme', true), tenant: null });
		mocks.listMemberships.mockResolvedValue([]);
		const user = userEvent.setup();
		render(TenantSwitcher);

		await user.click(screen.getByRole('button', { name: /Switch organization/ }));
		await waitFor(() => expect(screen.getByText('No organizations')).toBeInTheDocument());
		expect(screen.getByRole('menuitem', { name: 'Platform administration' })).toBeInTheDocument();
	});

	it('names the platform mode on the button and marks no organization in it', async () => {
		setSession(session('t_01', 'acme', true));
		const user = userEvent.setup();
		render(TenantSwitcher, { platformMode: true });

		const button = screen.getByRole('button', { name: /Switch organization/ });
		expect(button).toHaveTextContent('Platform administration');
		expect(button).not.toHaveTextContent('acme');
		expect(button.querySelector('svg[data-icon="shield"]')).not.toBeNull();

		await user.click(button);
		const entry = await screen.findByRole('menuitem', { name: 'Platform administration' });
		expect(entry).toHaveAttribute('aria-current', 'page');
		expect(screen.getByRole('menu')).toHaveAttribute(
			'aria-activedescendant',
			'tenant-switch-platform'
		);
		for (const item of screen.getAllByRole('menuitemradio')) {
			expect(item).toHaveAttribute('aria-checked', 'false');
		}
	});

	it('leaves the platform mode for the current organization without a switch', async () => {
		setSession(session('t_01', 'acme', true));
		const user = userEvent.setup();
		render(TenantSwitcher, { platformMode: true });

		await user.click(screen.getByRole('button', { name: /Switch organization/ }));
		await user.click(await screen.findByRole('menuitemradio', { name: /acme/ }));

		await waitFor(() => expect(mocks.goto).toHaveBeenCalledWith('/(app)'));
		expect(mocks.switchTenant).not.toHaveBeenCalled();
	});

	it('leaves the platform mode for another organization with a switch', async () => {
		setSession(session('t_01', 'acme', true));
		const user = userEvent.setup();
		render(TenantSwitcher, { platformMode: true });

		await user.click(screen.getByRole('button', { name: /Switch organization/ }));
		await user.click(await screen.findByRole('menuitemradio', { name: /globex/ }));

		await waitFor(() => expect(mocks.switchTenant).toHaveBeenCalledWith('t_02'));
		await waitFor(() => expect(mocks.goto).toHaveBeenCalledWith('/(app)'));
	});
});
