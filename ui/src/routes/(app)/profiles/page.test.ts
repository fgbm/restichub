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
	listPlans: vi.fn(),
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
			listPlans: mocks.listPlans
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
			retention: { keep_daily: 7, keep_weekly: 4 },
			paths: { all: ['/srv/db', '/etc'] },
			updated_at: '2026-10-01T00:00:00Z'
		}
	]);
	mocks.listRepositories.mockResolvedValue([{ id: 'r_1', name: 'offsite' }]);
	mocks.listPlans.mockResolvedValue([{ id: 'pl_1', profile_id: 'p_1' }]);
});

describe('Profiles list', () => {
	it('links the profile name to the editor page', async () => {
		render(Profiles);
		const link = await screen.findByRole('link', { name: 'db-nightly' });
		expect(link).toHaveAttribute('href', '/(app)/profiles/p_1');
	});

	it('keeps copy and delete as icon buttons and blocks the delete of a profile in use', async () => {
		render(Profiles);
		await screen.findByRole('link', { name: 'db-nightly' });
		expect(screen.getByRole('button', { name: 'Make a copy' })).toBeInTheDocument();
		const remove = screen.getByRole('button', { name: 'Delete profile' });
		expect(remove).toBeDisabled();
		expect(remove).toHaveAttribute('title', 'A plan uses the profile. Delete the plans first.');
		expect(screen.queryByRole('button', { name: 'More actions for db-nightly' })).toBeNull();
		expect(screen.getByText('offsite')).toBeInTheDocument();
		expect(screen.getByText('1 plan')).toBeInTheDocument();
		expect(screen.getByText('/srv/db +1')).toBeInTheDocument();
		expect(screen.getByText('7 d · 4 wk')).toBeInTheDocument();
	});

	it('opens a new profile as a copy', async () => {
		const user = userEvent.setup();
		render(Profiles);
		await user.click(await screen.findByRole('button', { name: 'Make a copy' }));
		expect(mocks.goto).toHaveBeenCalledWith('/(app)/profiles/new?from=p_1');
	});

	it('marks a profile without plans as not used', async () => {
		mocks.listPlans.mockResolvedValue([]);
		render(Profiles);
		await screen.findByRole('link', { name: 'db-nightly' });
		expect(screen.getByText('Not used', { selector: '.status' })).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Delete profile' })).toBeEnabled();
	});

	it('opens the new profile page from the Add button', async () => {
		const user = userEvent.setup();
		render(Profiles);
		await user.click(await screen.findByRole('button', { name: 'Add profile' }));
		expect(mocks.goto).toHaveBeenCalledWith('/(app)/profiles/new');
	});
});
