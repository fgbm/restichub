/*
 * The Profiles list links each profile to its editor page (FR-PF-01).
 */
import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Profiles from './+page.svelte';

const mocks = vi.hoisted(() => ({
	listProfiles: vi.fn(),
	listRepositories: vi.fn(),
	listAgents: vi.fn(),
	goto: vi.fn()
}));

vi.mock('$app/paths', () => ({
	resolve: (path: string, params?: Record<string, string>) =>
		params ? path.replace('[id]', params.id) : path
}));
vi.mock('$app/navigation', () => ({ goto: mocks.goto }));
vi.mock('#lib/stores/session.svelte.ts', () => ({
	getSession: () => ({
		user: { id: 'u_01', username: 'admin', display_name: 'Anna', role: 'admin' },
		tenant: { id: 't_01', name: 'acme' },
		role: 'admin'
	})
}));
vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		service: {
			...actual.service,
			listProfiles: mocks.listProfiles,
			listRepositories: mocks.listRepositories,
			listAgents: mocks.listAgents
		}
	};
});

beforeEach(() => {
	for (const mock of Object.values(mocks)) mock.mockReset();
	mocks.listProfiles.mockResolvedValue([
		{
			id: 'p_1',
			name: 'db-nightly',
			repository_id: 'r_1',
			tags: ['db'],
			retention: null
		}
	]);
	mocks.listRepositories.mockResolvedValue([{ id: 'r_1', name: 'offsite' }]);
	mocks.listAgents.mockResolvedValue([]);
});

describe('Profiles list', () => {
	it('links the profile name to the editor page', async () => {
		render(Profiles);
		const link = await screen.findByRole('link', { name: 'db-nightly' });
		expect(link).toHaveAttribute('href', '/(app)/profiles/p_1');
	});

	it('opens the editor page from the Edit action', async () => {
		const user = userEvent.setup();
		render(Profiles);
		await user.click(await screen.findByRole('button', { name: 'More actions for db-nightly' }));
		await user.click(await screen.findByText('Edit'));
		expect(mocks.goto).toHaveBeenCalledWith('/(app)/profiles/p_1');
	});

	it('opens the new profile page from the Add button', async () => {
		const user = userEvent.setup();
		render(Profiles);
		await user.click(await screen.findByRole('button', { name: 'Add profile' }));
		expect(mocks.goto).toHaveBeenCalledWith('/(app)/profiles/new');
	});
});
