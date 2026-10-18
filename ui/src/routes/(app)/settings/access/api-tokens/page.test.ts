/*
 * The API tokens screen lists the valid tokens first and revokes one (FR-API-01). A role below
 * Admin gets no token action, because the API refuses every API token operation of that role
 * (AC-AU-02).
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '#lib/api/index.ts';
import Tokens from './+page.svelte';

const mocks = vi.hoisted(() => ({
	listTokens: vi.fn(),
	revokeToken: vi.fn(),
	pushToast: vi.fn(),
	goto: vi.fn()
}));

const session = vi.hoisted(() => ({ role: 'admin' }));

const route = vi.hoisted(() => ({ url: new URL('http://localhost/settings/access/api-tokens') }));

vi.mock('$app/state', () => ({ page: route }));
vi.mock('$app/navigation', () => ({
	goto: (address: string) => {
		mocks.goto(address);
		route.url = new URL(address, 'http://localhost');
	}
}));
vi.mock('#lib/stores/session.svelte.ts', () => ({
	getSession: () => ({
		user: { id: 'u_01', username: 'admin', display_name: 'Anna', role: session.role },
		tenant: { id: 't_01', name: 'acme' },
		role: session.role
	}),
	hasRole: (...roles: string[]) => roles.includes(session.role)
}));
vi.mock('#lib/stores/toasts.svelte.ts', () => ({ pushToast: mocks.pushToast }));
vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		service: { ...actual.service, listTokens: mocks.listTokens, revokeToken: mocks.revokeToken }
	};
});

const DAY = 86_400_000;

function token(overrides: Record<string, unknown> = {}) {
	return {
		id: 'tk_1',
		name: 'ci-deploy',
		display_prefix: 'rh_7Kq2',
		role: 'operator',
		created_at: '2026-01-01T00:00:00Z',
		expires_at: null,
		last_used_at: null,
		revoked_at: null,
		...overrides
	};
}

beforeEach(() => {
	route.url = new URL('http://localhost/settings/access/api-tokens');
	session.role = 'admin';
	for (const mock of Object.values(mocks)) mock.mockReset();
	mocks.listTokens.mockResolvedValue([
		token(),
		token({
			id: 'tk_2',
			name: 'grafana-reader',
			expires_at: new Date(Date.now() + 9 * DAY).toISOString()
		}),
		token({ id: 'tk_3', name: 'old-sync', revoked_at: '2026-02-01T00:00:00Z' })
	]);
	mocks.revokeToken.mockResolvedValue(undefined);
});

describe('API tokens screen', () => {
	it('shows the valid tokens with the prefix, the role and the expiry', async () => {
		render(Tokens);
		await screen.findByText('ci-deploy');
		expect(screen.getByRole('button', { name: 'Valid 2' })).toHaveAttribute('aria-pressed', 'true');
		expect(screen.getByRole('button', { name: 'Expired and revoked 1' })).toBeInTheDocument();
		expect(screen.queryByText('old-sync')).toBeNull();
		expect(screen.getAllByText('rh_7Kq2…')).toHaveLength(2);
		expect(screen.getByText('no expiry')).toBeInTheDocument();
		expect(screen.getByText('Expires soon', { selector: '.status' })).toBeInTheDocument();
		expect(screen.getByText(/^expires in \d+ d$/)).toBeInTheDocument();
	});

	it('revokes a valid token after the confirmation', async () => {
		const user = userEvent.setup();
		render(Tokens);
		const item = (await screen.findByText('ci-deploy')).closest('li') as HTMLElement;
		await user.click(within(item).getByRole('button', { name: 'Revoke' }));
		const dialog = await screen.findByRole('dialog');
		await user.click(within(dialog).getByRole('button', { name: 'Revoke' }));
		await waitFor(() => expect(mocks.revokeToken).toHaveBeenCalledWith('tk_1'));
	});

	// The UI hides what the API refuses: an Admin manages the tokens (ADR-0023, AC-AU-02).
	it.each(['operator', 'viewer'] as const)(
		'ac_au_02_hides_the_create_button_from_a_%s_when_the_api_refuses_the_list',
		async (role) => {
			session.role = role;
			mocks.listTokens.mockRejectedValue(
				new ApiError({
					status: 403,
					code: 'forbidden',
					title: 'You do not have permission for this operation.'
				})
			);
			render(Tokens);

			expect(
				await screen.findByText(/You do not have permission for this operation/)
			).toBeInTheDocument();
			expect(screen.queryByRole('button', { name: 'Create token' })).toBeNull();
		}
	);

	it.each(['operator', 'viewer'] as const)(
		'ac_au_02_hides_the_revoke_action_from_a_%s',
		async (role) => {
			session.role = role;
			render(Tokens);

			await screen.findByText('ci-deploy');
			expect(screen.queryByRole('button', { name: 'Create token' })).toBeNull();
			expect(screen.queryByRole('button', { name: 'Revoke' })).toBeNull();
			// The token stays readable: its name opens the side panel.
			await userEvent.setup().click(screen.getByRole('button', { name: 'ci-deploy' }));
			expect(await screen.findByRole('dialog')).toBeInTheDocument();
		}
	);

	it('shows the create button to an admin', async () => {
		render(Tokens);

		await screen.findByText('ci-deploy');
		expect(screen.getByRole('button', { name: 'Create token' })).toBeInTheDocument();
	});
});
