/*
 * The Restore screen lists the snapshots of a repository, shows the tree of one snapshot and starts
 * a restore (FR-RS-01 to FR-RS-04).
 *
 * The test replaces the session store, the navigation and the service, so it renders the screen
 * alone. Only an Admin and an Operator use the screen.
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Restore from './+page.svelte';

const mocks = vi.hoisted(() => ({
	listRepositories: vi.fn(),
	listAgents: vi.fn(),
	getSnapshots: vi.fn(),
	startSnapshots: vi.fn(),
	startLs: vi.fn(),
	getLs: vi.fn(),
	restoreSnapshot: vi.fn(),
	goto: vi.fn(),
	pushToast: vi.fn()
}));

const session = vi.hoisted(() => ({ role: 'operator' as string }));

vi.mock('$app/navigation', () => ({ goto: mocks.goto }));
vi.mock('$app/paths', () => ({
	resolve: (path: string, params?: { id: string }) => path.replace('[id]', params?.id ?? '')
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
			getSnapshots: mocks.getSnapshots,
			startSnapshots: mocks.startSnapshots,
			startLs: mocks.startLs,
			getLs: mocks.getLs,
			restoreSnapshot: mocks.restoreSnapshot
		}
	};
});

const repository = {
	id: 'r_1',
	name: 'offsite-s3',
	backend: { kind: 's3', endpoint: 's3.example.net', bucket: 'b' },
	credentials: { password: true, s3_access_key: true, s3_secret_key: true, rest_password: false },
	initialized: true,
	maintenance_agent_id: 'a_1',
	created_at: '2026-10-01T00:00:00Z',
	updated_at: '2026-10-01T00:00:00Z'
};

const agent = {
	id: 'a_1',
	hostname: 'nas-backup',
	os: 'linux',
	arch: 'amd64',
	agent_version: '0.4.2',
	restic_version: '0.19.0',
	labels: [],
	restore_roots: ['/var/lib/restichub-restore'],
	status: 'active',
	last_seen_at: '2026-10-15T00:00:00Z',
	created_at: '2026-09-01T00:00:00Z',
	certificate_expires_at: '2027-09-01T00:00:00Z',
	certificate_serial: '0f01',
	revoked_at: null
};

function snapshot(id: string, host: string) {
	return {
		id: id.padEnd(64, '0'),
		short_id: id,
		time: '2026-10-14T12:00:00Z',
		hostname: host,
		username: 'root',
		tags: ['daily'],
		paths: ['/srv'],
		program_version: 'restic 0.18.1',
		total_files: 10,
		total_bytes: 1024
	};
}

function listing(state: string, snapshots: ReturnType<typeof snapshot>[] = []) {
	return {
		state,
		job_id: 'b_1',
		agent_id: 'a_1',
		fetched_at: '2026-10-15T00:00:00Z',
		age_seconds: 0,
		truncated: false,
		hosts: [],
		tags: [],
		paths: [],
		error_code: null,
		snapshots
	};
}

function node(name: string, kind: 'dir' | 'file', parent = '') {
	return {
		name,
		kind,
		path: `${parent}/${name}`,
		size: kind === 'file' ? 2048 : 0,
		permissions: kind === 'dir' ? 'drwxr-xr-x' : '-rw-r--r--',
		mtime: '2026-10-14T12:00:00Z',
		link_target: ''
	};
}

function page(nodes: ReturnType<typeof node>[], path = '') {
	return {
		state: 'done',
		job_id: 'l_1',
		snapshot_id: 'a1b2c3d4'.padEnd(64, '0'),
		path,
		offset: 0,
		limit: 1000,
		fetched_at: '2026-10-15T00:00:00Z',
		cached: false,
		last: true,
		next_offset: null,
		error_code: null,
		nodes
	};
}

beforeEach(() => {
	session.role = 'operator';
	vi.clearAllMocks();
	mocks.listRepositories.mockResolvedValue([repository]);
	mocks.listAgents.mockResolvedValue([agent]);
	mocks.getSnapshots.mockResolvedValue(
		listing('done', [snapshot('a1b2c3d4', 'nas-backup'), snapshot('b2c3d4e5', 'ws-anna')])
	);
	mocks.startLs.mockResolvedValue(page([node('srv', 'dir'), node('notes.txt', 'file')]));
});

describe('Restore screen', () => {
	it('fr_rs_01_lists_the_stored_snapshots_and_runs_a_new_request_with_the_filters', async () => {
		const user = userEvent.setup();
		render(Restore);
		expect(await screen.findByRole('button', { name: 'Open snapshot a1b2c3d4' })).toBeVisible();
		expect(screen.getByRole('button', { name: 'Open snapshot b2c3d4e5' })).toBeVisible();

		mocks.startSnapshots.mockResolvedValue(listing('done', [snapshot('b2c3d4e5', 'ws-anna')]));
		await user.type(screen.getByLabelText('Host'), 'ws-anna');
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
			expect(screen.queryByRole('button', { name: 'Open snapshot a1b2c3d4' })).toBeNull()
		);
	});

	it('fr_rs_01_a_repository_without_a_list_starts_a_request_at_once', async () => {
		mocks.getSnapshots.mockResolvedValue(listing('empty'));
		mocks.startSnapshots.mockResolvedValue(listing('done', [snapshot('a1b2c3d4', 'nas-backup')]));
		render(Restore);
		expect(await screen.findByRole('button', { name: 'Open snapshot a1b2c3d4' })).toBeVisible();
		expect(mocks.startSnapshots).toHaveBeenCalledWith('r_1', { hosts: [], tags: [], paths: [] });
	});

	it('fr_rs_01_shows_the_reason_when_the_list_job_fails', async () => {
		mocks.getSnapshots.mockResolvedValue({
			...listing('failed'),
			error_code: 'agent_capability_missing'
		});
		render(Restore);
		expect(await screen.findByText(/The snapshot list failed/)).toBeVisible();
	});

	it('fr_rs_02_loads_the_root_of_a_snapshot_and_a_directory_on_request', async () => {
		const user = userEvent.setup();
		render(Restore);
		await user.click(await screen.findByRole('button', { name: 'Open snapshot a1b2c3d4' }));
		const tree = await screen.findByRole('tree', { name: 'Files of the snapshot' });
		expect(await within(tree).findByText('notes.txt')).toBeVisible();
		expect(mocks.startLs).toHaveBeenCalledWith(
			'r_1',
			expect.objectContaining({ path: '', offset: 0, limit: 1000 })
		);

		mocks.startLs.mockResolvedValue(page([node('data', 'dir', '/srv')], '/srv'));
		await user.click(within(tree).getByRole('button', { name: 'Expand srv' }));
		expect(await within(tree).findByText('data')).toBeVisible();
		expect(mocks.startLs).toHaveBeenLastCalledWith(
			'r_1',
			expect.objectContaining({ path: '/srv' })
		);
	});

	it('fr_rs_04_starts_a_restore_with_the_selected_paths_and_warns_about_a_system_path', async () => {
		const user = userEvent.setup();
		mocks.restoreSnapshot.mockResolvedValue({ id: 'j_9' });
		render(Restore);
		await user.click(await screen.findByRole('button', { name: 'Open snapshot a1b2c3d4' }));
		const tree = await screen.findByRole('tree', { name: 'Files of the snapshot' });
		await user.click(await within(tree).findByRole('checkbox', { name: 'Select notes.txt' }));
		expect(screen.getByText('Selected: 1')).toBeVisible();

		await user.click(screen.getByRole('button', { name: 'Restore…' }));
		const dialog = await screen.findByRole('dialog');
		const field = within(dialog).getByLabelText(/Target path/);
		// The target starts as a folder in the first restore root of the host.
		expect(field).toHaveValue('/var/lib/restichub-restore/a1b2c3d4');
		expect(within(dialog).queryByText('System path')).toBeNull();
		await user.clear(field);
		await user.type(field, '/etc');
		expect(within(dialog).getByText('System path')).toBeVisible();
		expect(
			within(dialog).getByText('The target is outside the restore roots of the host.')
		).toBeVisible();
		expect(within(dialog).getByRole('button', { name: 'Start restore' })).toBeDisabled();
		await user.clear(field);
		await user.type(field, '/var/lib/restichub-restore/out');
		expect(within(dialog).queryByText('System path')).toBeNull();
		await user.click(within(dialog).getByRole('checkbox', { name: /Dry run/ }));
		expect(within(dialog).getByRole('checkbox', { name: /Verify after/ })).toBeDisabled();
		await user.click(within(dialog).getByRole('button', { name: 'Start restore' }));

		await waitFor(() =>
			expect(mocks.restoreSnapshot).toHaveBeenCalledWith('r_1', {
				agent_id: 'a_1',
				snapshot_id: 'a1b2c3d4'.padEnd(64, '0'),
				includes: ['/notes.txt'],
				excludes: [],
				target: '/var/lib/restichub-restore/out',
				overwrite: 'never',
				dry_run: true,
				verify: false
			})
		);
		await waitFor(() => expect(mocks.goto).toHaveBeenCalledWith('/(app)/jobs/j_9'));
	});

	it('fr_rs_04_refuses_a_relative_target_path', async () => {
		const user = userEvent.setup();
		render(Restore);
		await user.click(await screen.findByRole('button', { name: 'Open snapshot a1b2c3d4' }));
		await user.click(screen.getByRole('button', { name: 'Restore…' }));
		const dialog = await screen.findByRole('dialog');
		const field = within(dialog).getByLabelText(/Target path/);
		await user.clear(field);
		await user.type(field, 'restore');
		expect(within(dialog).getByText('Enter an absolute path.')).toBeVisible();
		expect(within(dialog).getByRole('button', { name: 'Start restore' })).toBeDisabled();
	});

	it('ac_au_02_a_viewer_gets_no_restore_screen', async () => {
		session.role = 'viewer';
		render(Restore);
		expect(await screen.findByText('Restore needs the Operator role or higher.')).toBeVisible();
		expect(mocks.startSnapshots).not.toHaveBeenCalled();
	});

	it('fr_rs_01_a_tenant_without_an_initialized_repository_sees_an_empty_state', async () => {
		mocks.listRepositories.mockResolvedValue([{ ...repository, initialized: false }]);
		render(Restore);
		expect(await screen.findByText('No repository to restore from')).toBeVisible();
	});
});
