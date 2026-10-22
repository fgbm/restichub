/*
 * The Profiles list links each profile to its editor page (FR-PF-01).
 */
import { render, screen, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Profiles from './+page.svelte';

const mocks = vi.hoisted(() => ({
	listProfiles: vi.fn(),
	listPlans: vi.fn(),
	profileUsage: vi.fn(),
	deleteProfile: vi.fn(),
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
			listPlans: mocks.listPlans,
			profileUsage: mocks.profileUsage,
			deleteProfile: mocks.deleteProfile
		}
	};
});

beforeEach(() => {
	for (const mock of Object.values(mocks)) mock.mockReset();
	mocks.listProfiles.mockResolvedValue([
		{
			id: 'p_1',
			name: 'db-nightly',
			tags: ['db'],
			retention: { keep_daily: 7, keep_weekly: 4 },
			paths: { all: ['/srv/db', '/etc'] },
			updated_at: '2026-10-01T00:00:00Z'
		}
	]);
	mocks.listPlans.mockResolvedValue([{ id: 'pl_1', profile_id: 'p_1' }]);
});

describe('Profiles list', () => {
	it('links the profile name to the editor page', async () => {
		render(Profiles);
		const link = await screen.findByRole('link', { name: 'db-nightly' });
		expect(link).toHaveAttribute('href', '/(app)/profiles/p_1');
	});

	it('keeps copy and delete as icon buttons and explains the delete of a profile in use', async () => {
		render(Profiles);
		await screen.findByRole('link', { name: 'db-nightly' });
		expect(screen.getByRole('button', { name: 'Make a copy' })).toBeInTheDocument();
		const remove = screen.getByRole('button', { name: 'Delete profile' });
		expect(remove).toBeEnabled();
		expect(remove).toHaveAttribute('title', 'A plan uses the profile. Delete the plans first.');
		expect(screen.queryByRole('button', { name: 'More actions for db-nightly' })).toBeNull();
		expect(screen.getByText('1 plan')).toBeInTheDocument();
		expect(screen.getByText('/srv/db +1')).toBeInTheDocument();
		expect(screen.getByText('7 d · 4 wk')).toBeInTheDocument();
	});

	it('adr_0047 shows the paths, the retention and the plans without a repository', async () => {
		render(Profiles);

		const link = await screen.findByRole('link', { name: 'db-nightly' });
		const item = link.closest('li') as HTMLElement;
		expect(item.querySelector('.item__meta')).toHaveTextContent('/srv/db +1 · 7 d · 4 wk · 1 plan');
		expect(screen.queryByRole('button', { name: /^Repository/ })).toBeNull();
		expect(screen.getByRole('button', { name: /^Tags/ })).toBeInTheDocument();
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

	it('names the plans that block the delete and disables the confirm button', async () => {
		const user = userEvent.setup();
		mocks.profileUsage.mockResolvedValue({
			blocked_by: 'profile_in_use',
			blockers: { items: [{ kind: 'plan', id: 'pl_1', name: 'Nightly' }], total: 1 }
		});
		render(Profiles);
		await user.click(await screen.findByRole('button', { name: 'Delete profile' }));

		const dialog = await screen.findByRole('dialog', { name: 'Delete the profile?' });
		expect(await within(dialog).findByRole('alert')).toHaveTextContent('Plans use this profile.');
		expect(within(dialog).getByRole('link', { name: 'Nightly' })).toHaveAttribute(
			'href',
			'/(app)/plans/pl_1'
		);
		expect(within(dialog).getByRole('button', { name: 'Delete profile' })).toBeDisabled();
	});

	it('shows the server reason when the state changes before the click', async () => {
		const user = userEvent.setup();
		const { ApiError } = await import('#lib/api/errors.ts');
		mocks.profileUsage.mockResolvedValue({
			blocked_by: null,
			blockers: { items: [], total: 0 }
		});
		mocks.deleteProfile.mockRejectedValue(
			new ApiError({
				status: 409,
				code: 'profile_in_use',
				title: 'Conflict',
				blockers: { items: [{ kind: 'plan', id: 'pl_9', name: 'Late plan' }], total: 1 }
			})
		);
		render(Profiles);
		await user.click(await screen.findByRole('button', { name: 'Delete profile' }));

		const dialog = await screen.findByRole('dialog', { name: 'Delete the profile?' });
		const confirm = await within(dialog).findByRole('button', { name: 'Delete profile' });
		await vi.waitFor(() => expect(confirm).toBeEnabled());
		await user.click(confirm);

		expect(await within(dialog).findByRole('link', { name: 'Late plan' })).toBeInTheDocument();
		expect(within(dialog).getByRole('alert')).toHaveTextContent('Plans use this profile.');
		expect(within(dialog).getByRole('button', { name: 'Delete profile' })).toBeDisabled();
	});
});

describe('the count of the parameters of a profile (FR-PF-09, ADR-0047)', () => {
	it('puts the count in a chip before the first field of line 2', async () => {
		mocks.listProfiles.mockResolvedValue([
			{
				id: 'p_1',
				name: 'db-nightly',
				tags: [],
				retention: null,
				paths: { all: ['/srv/db'] },
				parameters: [
					{
						name: 'pg_dir',
						kind: 'text',
						required: true,
						default_value: '/var/backups/pg',
						description: null,
						hidden: false
					}
				],
				updated_at: '2026-10-01T00:00:00Z'
			},
			{
				id: 'p_2',
				name: 'files',
				tags: [],
				retention: null,
				paths: { all: ['/srv/files'] },
				parameters: [],
				updated_at: '2026-10-01T00:00:00Z'
			}
		]);
		mocks.listPlans.mockResolvedValue([]);
		render(Profiles);
		await screen.findByRole('link', { name: 'db-nightly' });

		const chip = screen.getByText('1 parameter');
		expect(chip.closest('.item__chip')).not.toBeNull();
		expect(chip.closest('.item__chip')?.getAttribute('title')).toBe(
			'The profile declares parameters'
		);
		// A profile without a parameter shows no chip.
		expect(screen.queryByText('0 parameters')).not.toBeInTheDocument();
	});
});
