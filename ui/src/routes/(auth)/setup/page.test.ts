/*
 * The setup screen shows the one-time setup token field in both modes (ADR-0024).
 *
 * The token protects the tenant that the setup creates, so the field is not
 * hidden in single mode. The test replaces the SvelteKit modules and the service.
 */
import { render, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Setup from './+page.svelte';

const mocks = vi.hoisted(() => ({ instance: vi.fn(), setup: vi.fn() }));

vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));

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
	});

	it('shows the setup token field in single mode', async () => {
		mocks.instance.mockResolvedValue({ tenancy_mode: 'single', setup_required: true });
		const { container } = render(Setup);
		await waitFor(() => expect(container.querySelector('#setup-token')).not.toBeNull());
	});

	it('shows the setup token field in multi mode', async () => {
		mocks.instance.mockResolvedValue({ tenancy_mode: 'multi', setup_required: true });
		const { container } = render(Setup);
		await waitFor(() => expect(container.querySelector('#setup-token')).not.toBeNull());
	});
});
