/*
 * The sign-in screen asks for the username and the password only (ADR-0025) and sends
 * a signed-in person to the dashboard (#59).
 *
 * A fresh module per test resets the shared session store. The test replaces the
 * SvelteKit modules and the service, so it renders the screen alone.
 */
import { waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
	instance: vi.fn(),
	signIn: vi.fn(),
	currentUser: vi.fn(),
	goto: vi.fn(),
	url: 'http://localhost/sign-in'
}));

vi.mock('$app/navigation', () => ({ goto: mocks.goto }));
vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));
vi.mock('$app/state', () => ({
	page: {
		get url() {
			return new URL(mocks.url);
		}
	}
}));

vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		service: {
			...actual.service,
			instance: mocks.instance,
			signIn: mocks.signIn,
			currentUser: mocks.currentUser
		}
	};
});

function me() {
	return {
		user: {
			id: 'u_01',
			username: 'admin',
			display_name: 'Admin',
			role: 'admin',
			disabled: false,
			shared: false
		},
		tenant: { id: 't_01', name: 'acme' },
		role: 'admin'
	};
}

async function renderSignIn() {
	const { render } = await import('@testing-library/svelte');
	const { default: SignIn } = await import('./+page.svelte');
	return render(SignIn);
}

describe('the sign-in screen', () => {
	beforeEach(() => {
		vi.resetModules();
		mocks.instance.mockReset();
		mocks.signIn.mockReset();
		mocks.currentUser.mockReset();
		mocks.goto.mockReset();
		mocks.instance.mockResolvedValue({ tenancy_mode: 'multi', setup_required: false });
		mocks.currentUser.mockRejectedValue(new Error('anonymous'));
		mocks.url = 'http://localhost/sign-in';
	});

	it('asks for the username and the password only', async () => {
		const { container } = await renderSignIn();
		await waitFor(() => expect(container.querySelector('#signin-username')).not.toBeNull());
		expect(container.querySelector('#signin-password')).not.toBeNull();
		expect(container.querySelector('#signin-tenant')).toBeNull();
		expect(mocks.goto).not.toHaveBeenCalled();
	});

	it('has no tenant field in single mode either', async () => {
		mocks.instance.mockResolvedValue({ tenancy_mode: 'single', setup_required: false });
		const { container } = await renderSignIn();
		await waitFor(() => expect(container.querySelector('#signin-username')).not.toBeNull());
		expect(container.querySelector('#signin-tenant')).toBeNull();
	});

	it('sends a signed-in person to the dashboard', async () => {
		mocks.currentUser.mockResolvedValue(me());
		await renderSignIn();
		await waitFor(() => expect(mocks.goto).toHaveBeenCalledWith('/'));
	});

	it('honours the next parameter of a signed-in person', async () => {
		mocks.url = 'http://localhost/sign-in?next=%2Fusers';
		mocks.currentUser.mockResolvedValue(me());
		await renderSignIn();
		await waitFor(() => expect(mocks.goto).toHaveBeenCalledWith('/users'));
	});

	it('ignores a next value that leaves the site', async () => {
		mocks.url = 'http://localhost/sign-in?next=https%3A%2F%2Fexample.com';
		mocks.currentUser.mockResolvedValue(me());
		await renderSignIn();
		await waitFor(() => expect(mocks.goto).toHaveBeenCalledWith('/'));
	});

	it('sends a person to the setup screen while the setup is required', async () => {
		mocks.instance.mockResolvedValue({ tenancy_mode: 'multi', setup_required: true });
		await renderSignIn();
		await waitFor(() => expect(mocks.goto).toHaveBeenCalledWith('/(auth)/setup'));
	});
});
