/*
 * The setup screen shows the one-time setup token field in both modes (ADR-0024) and
 * leaves a configured installation (#58).
 *
 * The token protects the tenant that the setup creates, so the field is not hidden in
 * single mode. A fresh module per test resets the shared instance store. The test
 * replaces the SvelteKit modules and the service.
 */
import { waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ instance: vi.fn(), setup: vi.fn(), goto: vi.fn() }));

vi.mock('$app/navigation', () => ({ goto: mocks.goto }));
vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));

vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		service: { ...actual.service, instance: mocks.instance, setup: mocks.setup }
	};
});

async function renderSetup() {
	const { render } = await import('@testing-library/svelte');
	const { default: Setup } = await import('./+page.svelte');
	return render(Setup);
}

describe('the setup screen', () => {
	beforeEach(() => {
		vi.resetModules();
		mocks.instance.mockReset();
		mocks.goto.mockReset();
	});

	it('shows the setup token field in single mode', async () => {
		mocks.instance.mockResolvedValue({ tenancy_mode: 'single', setup_required: true });
		const { container } = await renderSetup();
		await waitFor(() => expect(container.querySelector('#setup-token')).not.toBeNull());
		expect(mocks.goto).not.toHaveBeenCalled();
	});

	it('shows the setup token field in multi mode', async () => {
		mocks.instance.mockResolvedValue({ tenancy_mode: 'multi', setup_required: true });
		const { container } = await renderSetup();
		await waitFor(() => expect(container.querySelector('#setup-token')).not.toBeNull());
	});

	it('sends the person to the sign-in screen when the setup is done', async () => {
		mocks.instance.mockResolvedValue({ tenancy_mode: 'multi', setup_required: false });
		await renderSetup();
		await waitFor(() => expect(mocks.goto).toHaveBeenCalledWith('/(auth)/sign-in'));
	});

	it('keeps the screen while the setup state is unknown', async () => {
		mocks.instance.mockRejectedValue(new Error('the control plane does not answer'));
		const { container } = await renderSetup();
		await waitFor(() => expect(mocks.instance).toHaveBeenCalled());
		expect(mocks.goto).not.toHaveBeenCalled();
		expect(container.querySelector('#setup-token')).not.toBeNull();
	});
});
