/*
 * The Restore page lists the snapshots of a repository with their filters (FR-RS-01). A snapshot
 * opens on its own page (restore-snapshot.test.ts).
 *
 * The test replaces the session store, the navigation and the service, so it renders the screen
 * alone. Only an Admin and an Operator use the screen.
 */
import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { agent, listing, repository, snapshot } from './restore-fixtures';
import Restore from './+page.svelte';

const mocks = vi.hoisted(() => ({
	listRepositories: vi.fn(),
	listAgents: vi.fn(),
	listProfiles: vi.fn(),
	getSnapshots: vi.fn(),
	startSnapshots: vi.fn(),
	startLs: vi.fn(),
	getLs: vi.fn(),
	restoreSnapshot: vi.fn(),
	goto: vi.fn(),
	pushToast: vi.fn()
}));

const session = vi.hoisted(() => ({ role: 'operator' as string }));
const route = vi.hoisted(() => ({
	url: new URL('http://localhost/restore'),
	params: {} as Record<string, string>
}));

vi.mock('$app/navigation', () => ({ goto: mocks.goto }));
vi.mock('$app/state', () => ({ page: route }));
vi.mock('$app/paths', () => ({
	resolve: (path: string, params: Record<string, string> = {}) =>
		path.replace(/\[(\w+)\]/g, (_, key: string) => params[key] ?? '')
}));
vi.mock('#lib/stores/toasts.svelte.ts', () => ({ pushToast: mocks.pushToast }));
vi.mock('#lib/stores/session.svelte.ts', () => ({
	getSession: () => ({
		user: { id: 'u_01', username: 'olga', display_name: 'Olga', role: session.role },
		tenant: { id: 't_01', name: 'acme' },
		role: session.role
	})
}));

vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		service: {
			...actual.service,
			listRepositories: mocks.listRepositories,
			listAgents: mocks.listAgents,
			listProfiles: mocks.listProfiles,
			getSnapshots: mocks.getSnapshots,
			startSnapshots: mocks.startSnapshots,
			startLs: mocks.startLs,
			getLs: mocks.getLs,
			restoreSnapshot: mocks.restoreSnapshot
		}
	};
});

beforeEach(() => {
	session.role = 'operator';
	vi.clearAllMocks();
	route.url = new URL('http://localhost/restore');
	mocks.listRepositories.mockResolvedValue([repository]);
	mocks.listAgents.mockResolvedValue([agent]);
	mocks.listProfiles.mockResolvedValue([]);
	mocks.getSnapshots.mockResolvedValue(
		listing('done', [snapshot('a1b2c3d4', 'nas-backup'), snapshot('b2c3d4e5', 'ws-anna')])
	);
});

describe('Restore page', () => {
	it('fr_rs_01_lists_the_stored_snapshots_and_runs_a_new_request_with_the_filters', async () => {
		const user = userEvent.setup();
		render(Restore);
		expect(await screen.findByRole('link', { name: 'Open snapshot a1b2c3d4' })).toBeVisible();
		expect(screen.getByRole('link', { name: 'Open snapshot b2c3d4e5' })).toBeVisible();

		mocks.startSnapshots.mockResolvedValue(listing('done', [snapshot('b2c3d4e5', 'ws-anna')]));
		await user.type(screen.getByLabelText('Snapshot host'), 'ws-anna');
		await user.type(screen.getByLabelText('Tag'), 'daily');
		await user.click(screen.getByRole('button', { name: 'Find snapshots' }));
		await waitFor(() =>
			expect(mocks.startSnapshots).toHaveBeenCalledWith('r_1', {
				hosts: ['ws-anna'],
				tags: ['daily'],
				paths: []
			})
		);
		await waitFor(() =>
			expect(screen.queryByRole('link', { name: 'Open snapshot a1b2c3d4' })).toBeNull()
		);
		// The address keeps the filters, so the back link of the snapshot page restores them.
		await waitFor(() =>
			expect(mocks.goto).toHaveBeenCalledWith(
				'/(app)/restore?host=ws-anna&tag=daily&repository=r_1',
				{ replaceState: true }
			)
		);
	});

	it('fr_rs_01_a_snapshot_links_to_its_own_page_with_the_filters', async () => {
		route.url = new URL('http://localhost/restore?repository=r_1&tag=daily');
		mocks.getSnapshots.mockResolvedValue({
			...listing('done', [snapshot('a1b2c3d4', 'nas-backup')]),
			tags: ['daily']
		});
		render(Restore);
		const link = await screen.findByRole('link', { name: 'Open snapshot a1b2c3d4' });
		expect(link).toHaveAttribute(
			'href',
			`/(app)/restore/r_1/${'a1b2c3d4'.padEnd(64, '0')}?tag=daily`
		);
		// The stored list already holds the tag filter, so no new request starts.
		expect(mocks.startSnapshots).not.toHaveBeenCalled();
	});

	it('fr_rs_01_a_filter_in_the_address_that_the_stored_list_lacks_starts_a_request', async () => {
		route.url = new URL('http://localhost/restore?repository=r_1&host=ws-anna');
		mocks.startSnapshots.mockResolvedValue(listing('done', [snapshot('b2c3d4e5', 'ws-anna')]));
		render(Restore);
		await waitFor(() =>
			expect(mocks.startSnapshots).toHaveBeenCalledWith('r_1', {
				hosts: ['ws-anna'],
				tags: [],
				paths: []
			})
		);
	});

	it('fr_rs_01_marks_the_snapshot_that_the_person_had_open', async () => {
		route.url = new URL(`http://localhost/restore?snapshot=${'b2c3d4e5'.padEnd(64, '0')}`);
		render(Restore);
		expect(await screen.findByRole('link', { name: 'Open snapshot b2c3d4e5' })).toHaveAttribute(
			'aria-current',
			'true'
		);
		expect(screen.getByRole('link', { name: 'Open snapshot a1b2c3d4' })).not.toHaveAttribute(
			'aria-current'
		);
	});

	it('fr_rs_01_reset_clears_every_filter_and_searches_again', async () => {
		const user = userEvent.setup();
		route.url = new URL('http://localhost/restore?repository=r_1&tag=daily&from=2026-10-01');
		mocks.startSnapshots.mockResolvedValue(listing('done', [snapshot('a1b2c3d4', 'nas-backup')]));
		render(Restore);
		await screen.findByRole('link', { name: 'Open snapshot a1b2c3d4' });
		expect(screen.getByLabelText('Tag')).toHaveValue('daily');
		mocks.startSnapshots.mockClear();
		await user.click(screen.getByRole('button', { name: 'Reset filters' }));
		expect(screen.getByLabelText('Tag')).toHaveValue('');
		expect(screen.getByLabelText('From')).toHaveValue('');
		await waitFor(() =>
			expect(mocks.startSnapshots).toHaveBeenCalledWith('r_1', { hosts: [], tags: [], paths: [] })
		);
	});

	it('fr_rs_01_a_repository_without_a_list_starts_a_request_at_once', async () => {
		mocks.getSnapshots.mockResolvedValue(listing('empty'));
		mocks.startSnapshots.mockResolvedValue(listing('done', [snapshot('a1b2c3d4', 'nas-backup')]));
		render(Restore);
		expect(await screen.findByRole('link', { name: 'Open snapshot a1b2c3d4' })).toBeVisible();
		expect(mocks.startSnapshots).toHaveBeenCalledWith('r_1', { hosts: [], tags: [], paths: [] });
	});

	it('fr_rs_01_the_failed_list_banner_links_to_the_job_log', async () => {
		mocks.getSnapshots.mockResolvedValue({
			...listing('failed'),
			error_code: 'agent_capability_missing'
		});
		render(Restore);
		expect(await screen.findByText(/The snapshot list failed/)).toBeVisible();
		expect(screen.getByRole('link', { name: 'Open the job log' })).toHaveAttribute(
			'href',
			'/(app)/jobs/b_1'
		);
	});

	it('fr_rs_01_shortens_an_automatic_profile_tag_and_keeps_the_full_value_in_the_title', async () => {
		const id = '0b9f4a2c-3e1d-4c55-8a21-9d7e6f5a4b3c';
		const other = '7c1e5d90-2a44-4f1b-9b0e-3d6a8c2f1e55';
		mocks.listProfiles.mockResolvedValue([{ id, name: 'Office files' }]);
		mocks.getSnapshots.mockResolvedValue(
			listing('done', [
				{
					...snapshot('a1b2c3d4', 'nas-backup'),
					tags: [`restichub-profile-${id}`, `restichub-profile-${other}`, 'daily']
				}
			])
		);
		render(Restore);
		const resolved = await screen.findByText('Profile Office files');
		expect(resolved.closest('[title]')).toHaveAttribute('title', `restichub-profile-${id}`);
		const unresolved = screen.getByText('Profile 7c1e5d90');
		expect(unresolved.closest('[title]')).toHaveAttribute('title', `restichub-profile-${other}`);
		expect(screen.getByText('daily')).toBeVisible();
	});

	it('fr_rs_01_shows_more_snapshots_on_request', async () => {
		const user = userEvent.setup();
		mocks.getSnapshots.mockResolvedValue(
			listing(
				'done',
				Array.from({ length: 20 }, (_, index) =>
					snapshot(`aa${String(index).padStart(6, '0')}`, 'nas-backup')
				)
			)
		);
		render(Restore);
		await screen.findByRole('link', { name: 'Open snapshot aa000000' });
		expect(screen.getAllByRole('link', { name: /^Open snapshot / })).toHaveLength(15);
		await user.click(screen.getByRole('button', { name: 'Show more (5 left)' }));
		expect(screen.getAllByRole('link', { name: /^Open snapshot / })).toHaveLength(20);
	});

	it('ac_au_02_a_viewer_gets_no_restore_screen', async () => {
		session.role = 'viewer';
		render(Restore);
		expect(await screen.findByText('Restore needs the Operator role or higher.')).toBeVisible();
		expect(mocks.startSnapshots).not.toHaveBeenCalled();
	});

	it('fr_rs_01_a_tenant_without_an_initialized_repository_sees_an_empty_state_with_a_link', async () => {
		const user = userEvent.setup();
		mocks.listRepositories.mockResolvedValue([{ ...repository, initialized: false }]);
		render(Restore);
		expect(await screen.findByText('No repository to restore from')).toBeVisible();
		await user.click(screen.getByRole('button', { name: 'Open repositories' }));
		expect(mocks.goto).toHaveBeenCalledWith('/(app)/repositories');
	});
});
