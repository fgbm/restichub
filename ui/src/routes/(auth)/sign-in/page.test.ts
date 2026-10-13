/*
 * The sign-in screen hides the tenant field in single mode (ADR-0024).
 *
 * The test replaces the SvelteKit modules and the service, so it renders the
 * screen alone. It sets the mode through the `/api/v1/instance` response.
 */
import { render, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import SignIn from './+page.svelte';

const mocks = vi.hoisted(() => ({
	instance: vi.fn(),
	signIn: vi.fn(),
	setupStatus: vi.fn()
}));

vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));
vi.mock('$app/state', () => ({ page: { url: new URL('http://localhost/sign-in') } }));

vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		service: {
			...actual.service,
			instance: mocks.instance,
			signIn: mocks.signIn,
			setupStatus: mocks.setupStatus
		}
	};
});

describe('the sign-in screen', () => {
	beforeEach(() => {
		mocks.instance.mockReset();
		mocks.setupStatus.mockResolvedValue({ setup_required: false });
	});

	it('hides the tenant field in single mode', async () => {
		mocks.instance.mockResolvedValue({ tenancy_mode: 'single', setup_required: false });
		const { container } = render(SignIn);
		await waitFor(() => expect(container.querySelector('#signin-tenant')).toBeNull());
	});

	it('shows the tenant field in multi mode', async () => {
		mocks.instance.mockResolvedValue({ tenancy_mode: 'multi', setup_required: false });
		const { container } = render(SignIn);
		await waitFor(() => expect(container.querySelector('#signin-tenant')).not.toBeNull());
	});
});
