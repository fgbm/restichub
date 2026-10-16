/*
 * The organizations screen of the platform lists the active organizations first, marks an
 * organization without an Admin and suspends one (FR-TN-02).
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Tenants from './+page.svelte';

const mocks = vi.hoisted(() => ({
	listTenants: vi.fn(),
	suspendTenant: vi.fn(),
	pushToast: vi.fn()
}));

const route = vi.hoisted(() => ({ url: new URL('http://localhost/platform/tenants') }));

vi.mock('$app/state', () => ({ page: route }));
vi.mock('$app/navigation', () => ({
	goto: (address: string) => {
		route.url = new URL(address, 'http://localhost');
	}
}));
vi.mock('#lib/stores/toasts.svelte.ts', () => ({ pushToast: mocks.pushToast }));
vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		service: {
			...actual.service,
			listTenants: mocks.listTenants,
			suspendTenant: mocks.suspendTenant
		}
	};
});

function tenant(overrides: Record<string, unknown> = {}) {
	return {
		id: 't_1',
		name: 'acme',
		status: 'active',
		admin_state: 'present',
		user_count: 7,
		agent_count: 24,
		created_at: '2026-01-12T00:00:00Z',
		...overrides
	};
}

beforeEach(() => {
	route.url = new URL('http://localhost/platform/tenants');
	for (const mock of Object.values(mocks)) mock.mockReset();
	mocks.listTenants.mockResolvedValue([
		tenant(),
		tenant({ id: 't_2', name: 'globex', admin_state: 'none', user_count: 2 }),
		tenant({ id: 't_3', name: 'northwind', status: 'suspended' })
	]);
	mocks.suspendTenant.mockResolvedValue(undefined);
});

describe('Organizations screen', () => {
	it('shows the active organizations and marks one without an Admin', async () => {
		render(Tenants);
		await screen.findByText('acme');
		expect(screen.getByRole('button', { name: 'Active 2' })).toHaveAttribute(
			'aria-pressed',
			'true'
		);
		expect(screen.getByRole('button', { name: 'Suspended 1' })).toBeInTheDocument();
		expect(screen.queryByText('northwind')).toBeNull();
		expect(screen.getByText('No administrator', { selector: '.status' })).toBeInTheDocument();
		expect(screen.getByText('7 users')).toBeInTheDocument();
		expect(screen.getAllByText('24 hosts')).toHaveLength(2);
	});

	it('suspends an active organization from its item', async () => {
		const user = userEvent.setup();
		render(Tenants);
		const item = (await screen.findByText('acme')).closest('li') as HTMLElement;
		await user.click(within(item).getByRole('button', { name: 'Suspend' }));
		await waitFor(() => expect(mocks.suspendTenant).toHaveBeenCalledWith('t_1'));
	});
});
