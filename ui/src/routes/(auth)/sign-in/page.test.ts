/*
 * The sign-in screen asks for the username and the password only (ADR-0025) and sends
 * a signed-in person to the dashboard (#59).
 *
 * The test replaces the SvelteKit modules, the session store, the instance store and
 * the service, so it renders the screen alone.
 */
import { render, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import SignIn from './+page.svelte';

const mocks = vi.hoisted(() => ({
	instance: vi.fn(),
	currentUser: vi.fn(),
	goto: vi.fn(),
	url: 'http://localhost/sign-in'
}));

const store = vi.hoisted(() => ({
	current: null as unknown,
	loaded: false,
	setupRequired: null as boolean | null
}));

vi.mock('$app/navigation', () => ({ goto: mocks.goto }));
vi.mock('$app/paths', () => ({
	resolve: (path: string) => path,
	asset: (path: string) => `/${path}`
}));
vi.mock('$app/state', () => ({
	page: {
		get url() {
			return new URL(mocks.url);
		}
	}
}));

vi.mock('#lib/stores/session.svelte.ts', () => ({
	getSession: () => store.current,
	isLoading: () => false,
	isLoaded: () => store.loaded,
	setSession: (value: unknown) => {
		store.current = value;
		store.loaded = true;
	},
	clearSession: () => {
		store.current = null;
		store.loaded = true;
	},
	loadSession: async () => {
		try {
			store.current = await mocks.currentUser();
		} catch {
			store.current = null;
		}
		store.loaded = true;
		return store.current;
	},
	getRole: () => (store.current as { role?: string } | null)?.role ?? null,
	hasRole: () => false
}));

vi.mock('#lib/stores/instance.svelte.ts', () => ({
	getTenancyMode: () => 'multi',
	isSingleTenant: () => false,
	getSetupRequired: () => store.setupRequired,
	isInstanceLoaded: () => true,
	loadInstance: async () => {
		try {
			store.setupRequired = (await mocks.instance()).setup_required;
		} catch {
			// The real store keeps the last value when the control plane does not answer.
		}
	}
}));

vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		service: { ...actual.service, instance: mocks.instance, currentUser: mocks.currentUser }
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

describe('the sign-in screen', () => {
	beforeEach(() => {
		mocks.instance.mockReset();
		mocks.currentUser.mockReset();
		mocks.goto.mockReset();
		store.current = null;
		store.loaded = false;
		store.setupRequired = null;
		mocks.instance.mockResolvedValue({ tenancy_mode: 'multi', setup_required: false });
		mocks.currentUser.mockRejectedValue(new Error('anonymous'));
		mocks.url = 'http://localhost/sign-in';
	});

	it('asks for the username and the password only', async () => {
		const { container } = render(SignIn);
		await waitFor(() => expect(container.querySelector('#signin-username')).not.toBeNull());
		expect(container.querySelector('#signin-password')).not.toBeNull();
		expect(container.querySelector('#signin-tenant')).toBeNull();
		expect(mocks.currentUser).toHaveBeenCalled();
		expect(mocks.goto).not.toHaveBeenCalled();
	});

	it('has no tenant field in single mode either', async () => {
		mocks.instance.mockResolvedValue({ tenancy_mode: 'single', setup_required: false });
		const { container } = render(SignIn);
		await waitFor(() => expect(container.querySelector('#signin-username')).not.toBeNull());
		expect(container.querySelector('#signin-tenant')).toBeNull();
	});

	it('sends a signed-in person to the dashboard', async () => {
		mocks.currentUser.mockResolvedValue(me());
		render(SignIn);
		await waitFor(() => expect(mocks.goto).toHaveBeenCalledWith('/'));
	});

	it('honours the next parameter of a signed-in person', async () => {
		mocks.url = 'http://localhost/sign-in?next=%2Fusers';
		mocks.currentUser.mockResolvedValue(me());
		render(SignIn);
		await waitFor(() => expect(mocks.goto).toHaveBeenCalledWith('/users'));
	});

	it('ignores a next value that leaves the site', async () => {
		mocks.url = 'http://localhost/sign-in?next=https%3A%2F%2Fexample.com';
		mocks.currentUser.mockResolvedValue(me());
		render(SignIn);
		await waitFor(() => expect(mocks.goto).toHaveBeenCalledWith('/'));
	});

	it('sends a person to the setup screen while the setup is required', async () => {
		mocks.instance.mockResolvedValue({ tenancy_mode: 'multi', setup_required: true });
		render(SignIn);
		await waitFor(() => expect(mocks.goto).toHaveBeenCalledWith('/(auth)/setup'));
	});
});
