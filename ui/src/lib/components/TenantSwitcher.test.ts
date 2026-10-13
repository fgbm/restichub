/*
 * The tenant switcher (ADR-0025): the menu, the roles, the check mark, the filter, the keyboard,
 * the reload on open, the error toast and the click outside.
 */
import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { setSession } from '#lib/stores/session.svelte.ts';
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

function session(id: string, name: string) {
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
		role: 'admin' as const
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

		await user.click(screen.getByRole('button', { name: 'Switch tenant' }));
		await waitFor(() => expect(screen.getByRole('menu')).toBeInTheDocument());

		const current = screen.getByRole('menuitemradio', { name: /acme/ });
		expect(current).toHaveAttribute('aria-checked', 'true');
		const other = screen.getByRole('menuitemradio', { name: /globex/ });
		expect(other).toHaveAttribute('aria-checked', 'false');
	});

	it('reloads the membership list on every open', async () => {
		const user = userEvent.setup();
		render(TenantSwitcher);
		const button = screen.getByRole('button', { name: 'Switch tenant' });

		await user.click(button);
		await waitFor(() => expect(mocks.listMemberships).toHaveBeenCalledTimes(1));
		await user.keyboard('{Escape}');
		await user.click(button);
		await waitFor(() => expect(mocks.listMemberships).toHaveBeenCalledTimes(2));
	});

	it('moves with the arrow keys and switches with Enter on the active item', async () => {
		const user = userEvent.setup();
		render(TenantSwitcher);

		await user.click(screen.getByRole('button', { name: 'Switch tenant' }));
		await waitFor(() => expect(screen.getByRole('menu')).toBeInTheDocument());

		await user.keyboard('{ArrowDown}');
		await user.keyboard('{Enter}');
		await waitFor(() => expect(mocks.switchTenant).toHaveBeenCalledWith('t_02'));
		await waitFor(() => expect(mocks.invalidateAll).toHaveBeenCalled());
	});

	it('closes on Escape and returns the focus to the button', async () => {
		const user = userEvent.setup();
		render(TenantSwitcher);

		const button = screen.getByRole('button', { name: 'Switch tenant' });
		await user.click(button);
		await waitFor(() => expect(screen.getByRole('menu')).toBeInTheDocument());

		await user.keyboard('{Escape}');
		await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument());
		expect(button).toHaveFocus();
	});

	it('closes on a click outside', async () => {
		const user = userEvent.setup();
		render(TenantSwitcher);

		await user.click(screen.getByRole('button', { name: 'Switch tenant' }));
		await waitFor(() => expect(screen.getByRole('menu')).toBeInTheDocument());

		await user.click(document.body);
		await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument());
	});

	it('shows an error toast when the switch fails', async () => {
		mocks.switchTenant.mockRejectedValue(new Error('boom'));
		const user = userEvent.setup();
		render(TenantSwitcher);

		await user.click(screen.getByRole('button', { name: 'Switch tenant' }));
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

		await user.click(screen.getByRole('button', { name: 'Switch tenant' }));
		const filter = await screen.findByRole('searchbox', { name: 'Filter tenants' });
		await user.type(filter, 'glob');
		await waitFor(() => expect(screen.queryByRole('menuitemradio', { name: /acme/ })).toBeNull());
		expect(screen.getByRole('menuitemradio', { name: /globex/ })).toBeInTheDocument();
	});

	it('shows a separate Platform entry for a Platform Admin', async () => {
		mocks.listMemberships.mockResolvedValue([
			...tenants,
			{ id: 't_platform', name: 'platform', role: 'platform_admin' as const }
		]);
		const user = userEvent.setup();
		render(TenantSwitcher);

		await user.click(screen.getByRole('button', { name: 'Switch tenant' }));
		await waitFor(() =>
			expect(screen.getByRole('menuitemradio', { name: /Platform/ })).toBeInTheDocument()
		);
	});
});
