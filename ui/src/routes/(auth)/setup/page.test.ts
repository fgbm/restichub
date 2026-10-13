/*
 * The setup screen shows the one-time setup token field in both modes (ADR-0024) and
 * leaves a configured installation (#58).
 *
 * The token protects the tenant that the setup creates, so the field is not hidden in
 * single mode. The test replaces the SvelteKit modules, the instance store and the service.
 */
import { render, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Setup from './+page.svelte';

const mocks = vi.hoisted(() => ({ instance: vi.fn(), setup: vi.fn(), goto: vi.fn() }));

const store = vi.hoisted(() => ({ single: false, setupRequired: null as boolean | null }));

vi.mock('$app/navigation', () => ({ goto: mocks.goto }));
vi.mock('$app/paths', () => ({
	resolve: (path: string) => path,
	asset: (path: string) => `/${path}`
}));

vi.mock('#lib/stores/instance.svelte.ts', () => ({
	getTenancyMode: () => (store.single ? 'single' : 'multi'),
	isSingleTenant: () => store.single,
	getSetupRequired: () => store.setupRequired,
	isInstanceLoaded: () => true,
	loadInstance: async () => {
		try {
			const instance = await mocks.instance();
			store.single = instance.tenancy_mode === 'single';
			store.setupRequired = instance.setup_required;
		} catch {
			// The real store keeps the last value when the control plane does not answer.
		}
	}
}));

vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		service: { ...actual.service, instance: mocks.instance, setup: mocks.setup }
	};
});

describe('the setup screen', () => {
	beforeEach(() => {
		mocks.instance.mockReset();
		mocks.goto.mockReset();
		store.single = false;
		store.setupRequired = null;
	});

	it('shows the setup token field in single mode', async () => {
		mocks.instance.mockResolvedValue({ tenancy_mode: 'single', setup_required: true });
		const { container } = render(Setup);
		await waitFor(() => expect(container.querySelector('#setup-token')).not.toBeNull());
		expect(mocks.goto).not.toHaveBeenCalled();
	});

	it('shows the setup token field in multi mode', async () => {
		mocks.instance.mockResolvedValue({ tenancy_mode: 'multi', setup_required: true });
		const { container } = render(Setup);
		await waitFor(() => expect(container.querySelector('#setup-token')).not.toBeNull());
	});

	it('sends the person to the sign-in screen when the setup is done', async () => {
		mocks.instance.mockResolvedValue({ tenancy_mode: 'multi', setup_required: false });
		render(Setup);
		await waitFor(() => expect(mocks.goto).toHaveBeenCalledWith('/(auth)/sign-in'));
	});

	it('keeps the screen while the setup state is unknown', async () => {
		mocks.instance.mockRejectedValue(new Error('the control plane does not answer'));
		const { container } = render(Setup);
		await waitFor(() => expect(mocks.instance).toHaveBeenCalled());
		expect(mocks.goto).not.toHaveBeenCalled();
		expect(container.querySelector('#setup-token')).not.toBeNull();
	});
});
