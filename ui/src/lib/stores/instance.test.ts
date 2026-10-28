/*
 * The tenancy mode of the installation: `/api/v1/instance` reports it, and a read that fails names
 * no mode. A screen never shows the default `multi` of the store as a fact (ADR-0055, decision 4).
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ instance: vi.fn() }));

vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return { ...actual, service: { ...actual.service, instance: mocks.instance } };
});

beforeEach(() => {
	vi.resetModules();
	mocks.instance.mockReset();
});

describe('adr_0055_the_tenancy_mode_is_a_fact_only_after_the_instance_route_answers', () => {
	it('keeps the mode unknown when the instance route fails', async () => {
		mocks.instance.mockRejectedValueOnce(new Error('offline'));
		const store = await import('./instance.svelte.ts');

		await store.loadInstance();

		expect(store.isInstanceLoaded()).toBe(true);
		expect(store.isTenancyModeKnown()).toBe(false);
		expect(store.getTenancyMode()).toBe('multi');
		// The multi-tenant surface stays, so the navigation and the sign-in screen keep their shape.
		expect(store.isSingleTenant()).toBe(false);
	});

	it('holds the mode of the installation when the instance route answers', async () => {
		mocks.instance.mockResolvedValueOnce({ tenancy_mode: 'single', setup_required: false });
		const store = await import('./instance.svelte.ts');

		await store.loadInstance();

		expect(store.isTenancyModeKnown()).toBe(true);
		expect(store.getTenancyMode()).toBe('single');
		expect(store.isSingleTenant()).toBe(true);
	});
});
