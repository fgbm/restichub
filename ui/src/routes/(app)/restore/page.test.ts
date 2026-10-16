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
	it('fr_rs_01_lists_the_stored_snapshots_and_filters_them_on_the_page_without_a_new_job', async () => {
		const user = userEvent.setup();
		mocks.getSnapshots.mockResolvedValue(
			listing('done', [
				{ ...snapshot('a1b2c3d4', 'nas-backup'), paths: ['/srv', '/etc'] },
				{ ...snapshot('b2c3d4e5', 'ws-anna'), paths: ['C:\\Users\\Anna'] }
			])
		);
		render(Restore);
		expect(await screen.findByRole('link', { name: /^Open snapshot a1b2c3d4/ })).toBeVisible();
		expect(screen.getByRole('link', { name: /^Open snapshot b2c3d4e5/ })).toBeVisible();

		await user.type(screen.getByRole('searchbox', { name: 'Search by path' }), 'c:\\users');
		await waitFor(() =>
			expect(screen.queryByRole('link', { name: /^Open snapshot a1b2c3d4/ })).toBeNull()
		);
		expect(screen.getByRole('link', { name: /^Open snapshot b2c3d4e5/ })).toBeVisible();
		expect(mocks.startSnapshots).not.toHaveBeenCalled();
		// The address keeps the filters, so the back link of the snapshot page restores them.
		await waitFor(() =>
			expect(mocks.goto).toHaveBeenCalledWith('/(app)/restore?path=c%3A%5Cusers&repository=r_1', {
				replace: true,
				reset: false
			})
		);
	});

	it('fr_rs_01_lists_the_newest_snapshot_first_with_the_count_next_to_the_title', async () => {
		mocks.getSnapshots.mockResolvedValue(
			listing('done', [
				{ ...snapshot('a1b2c3d4', 'nas-backup'), time: '2026-10-12T08:00:00Z' },
				{ ...snapshot('b2c3d4e5', 'ws-anna'), time: '2026-10-14T08:00:00Z' },
				{ ...snapshot('c3d4e5f6', 'ws-anna'), time: '2026-10-13T08:00:00Z' }
			])
		);
		const { container } = render(Restore);
		await screen.findByRole('link', { name: /^Open snapshot a1b2c3d4/ });
		const order = screen
			.getAllByRole('link', { name: /^Open snapshot / })
			.map((link) => link.getAttribute('aria-label') ?? '');
		expect(order[0]).toContain('b2c3d4e5');
		expect(order[1]).toContain('c3d4e5f6');
		expect(order[2]).toContain('a1b2c3d4');
		expect(container.querySelector('.page-header__count')).toHaveTextContent('3');
		expect(screen.queryByRole('heading', { name: 'Snapshots' })).toBeNull();
	});

	it('fr_rs_01_the_facet_popover_explains_how_the_filters_combine', async () => {
		const user = userEvent.setup();
		render(Restore);
		await screen.findByRole('link', { name: /^Open snapshot a1b2c3d4/ });
		expect(screen.queryByText('Any chosen value matches. All filters must match.')).toBeNull();
		await user.click(screen.getByRole('button', { name: 'Tag' }));
		expect(
			await screen.findByText('Any chosen value matches. All filters must match.')
		).toBeInTheDocument();
	});

	it('fr_rs_01_a_host_chosen_in_the_popover_shows_as_a_chip_and_narrows_the_list', async () => {
		const user = userEvent.setup();
		render(Restore);
		await screen.findByRole('link', { name: /^Open snapshot a1b2c3d4/ });
		await user.click(screen.getByRole('button', { name: 'Host' }));
		await user.click(await screen.findByLabelText('ws-anna'));
		expect(screen.getByText('Host: ws-anna')).toBeVisible();
		await waitFor(() =>
			expect(screen.queryByRole('link', { name: /^Open snapshot a1b2c3d4/ })).toBeNull()
		);
		await user.click(screen.getByRole('button', { name: 'Remove the filter Host: ws-anna' }));
		expect(await screen.findByRole('link', { name: /^Open snapshot a1b2c3d4/ })).toBeVisible();
	});

	it('fr_rs_01_a_filter_that_matches_nothing_offers_a_reset', async () => {
		const user = userEvent.setup();
		route.url = new URL('http://localhost/restore?repository=r_1&host=nobody');
		render(Restore);
		expect(await screen.findByText('Nothing found')).toBeVisible();
		await user.click(screen.getByRole('button', { name: 'Reset' }));
		expect(await screen.findByRole('link', { name: /^Open snapshot a1b2c3d4/ })).toBeVisible();
	});

	it('fr_rs_01_a_repository_without_snapshots_shows_its_own_empty_state', async () => {
		mocks.getSnapshots.mockResolvedValue(listing('done'));
		render(Restore);
		expect(await screen.findByText('This repository holds no snapshots')).toBeVisible();
	});

	it('fr_rs_01_the_refresh_button_runs_a_list_job_without_filters', async () => {
		const user = userEvent.setup();
		render(Restore);
		await screen.findByRole('link', { name: /^Open snapshot a1b2c3d4/ });
		mocks.startSnapshots.mockResolvedValue(listing('done', [snapshot('c3d4e5f6', 'nas-backup')]));
		await user.click(screen.getByRole('button', { name: 'Refresh the list from the repository' }));
		await waitFor(() =>
			expect(mocks.startSnapshots).toHaveBeenCalledWith('r_1', { hosts: [], tags: [], paths: [] })
		);
		expect(await screen.findByRole('link', { name: /^Open snapshot c3d4e5f6/ })).toBeVisible();
	});

	it('fr_rs_01_a_snapshot_links_to_its_own_page_with_the_filters', async () => {
		route.url = new URL('http://localhost/restore?repository=r_1&tag=daily');
		render(Restore);
		const link = await screen.findByRole('link', { name: /^Open snapshot a1b2c3d4/ });
		expect(link).toHaveAttribute(
			'href',
			`/(app)/restore/r_1/${'a1b2c3d4'.padEnd(64, '0')}?tag=daily`
		);
		// The page filters the whole stored list itself, so no new request starts.
		expect(mocks.startSnapshots).not.toHaveBeenCalled();
	});

	it('fr_rs_01_a_stored_list_made_with_filters_starts_a_whole_request', async () => {
		mocks.getSnapshots.mockResolvedValue({
			...listing('done', [snapshot('b2c3d4e5', 'ws-anna')]),
			hosts: ['ws-anna']
		});
		mocks.startSnapshots.mockResolvedValue(listing('done', [snapshot('b2c3d4e5', 'ws-anna')]));
		render(Restore);
		await waitFor(() =>
			expect(mocks.startSnapshots).toHaveBeenCalledWith('r_1', { hosts: [], tags: [], paths: [] })
		);
	});

	it('fr_rs_01_marks_the_snapshot_that_the_person_had_open', async () => {
		route.url = new URL(`http://localhost/restore?snapshot=${'b2c3d4e5'.padEnd(64, '0')}`);
		render(Restore);
		expect(await screen.findByRole('link', { name: /^Open snapshot b2c3d4e5/ })).toHaveAttribute(
			'aria-current',
			'true'
		);
		expect(screen.getByRole('link', { name: /^Open snapshot a1b2c3d4/ })).not.toHaveAttribute(
			'aria-current'
		);
	});

	it('fr_rs_01_a_repository_without_a_list_starts_a_request_at_once', async () => {
		mocks.getSnapshots.mockResolvedValue(listing('empty'));
		mocks.startSnapshots.mockResolvedValue(listing('done', [snapshot('a1b2c3d4', 'nas-backup')]));
		render(Restore);
		expect(await screen.findByRole('link', { name: /^Open snapshot a1b2c3d4/ })).toBeVisible();
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
		const tags = await screen.findByText('Profile Office files, Profile 7c1e5d90, daily');
		expect(tags).toHaveAttribute(
			'title',
			`restichub-profile-${id}, restichub-profile-${other}, daily`
		);
	});

	it('fr_rs_01_pages_the_snapshots', async () => {
		const user = userEvent.setup();
		mocks.getSnapshots.mockResolvedValue(
			listing(
				'done',
				Array.from({ length: 30 }, (_, index) =>
					snapshot(`aa${String(index).padStart(6, '0')}`, 'nas-backup')
				)
			)
		);
		render(Restore);
		await screen.findByRole('link', { name: /^Open snapshot aa000000/ });
		expect(screen.getAllByRole('link', { name: /^Open snapshot / })).toHaveLength(25);
		await user.click(screen.getByRole('button', { name: 'Next' }));
		expect(screen.getAllByRole('link', { name: /^Open snapshot / })).toHaveLength(5);
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
