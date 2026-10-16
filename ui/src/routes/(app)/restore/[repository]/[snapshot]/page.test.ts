/*
 * The snapshot page shows one snapshot, its file tree and the restore dialog (FR-RS-02 to FR-RS-04).
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { agent, listing, node, repository, snapshot, treePage } from '../../restore-fixtures';
import Snapshot from './+page.svelte';

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

const ID = 'a1b2c3d4'.padEnd(64, '0');
const peer = { ...agent, id: 'a_2', hostname: 'ws-anna' };

beforeEach(() => {
	session.role = 'operator';
	vi.clearAllMocks();
	route.url = new URL('http://localhost/restore/r_1/x?tag=daily&host=nas-backup');
	route.params = { repository: 'r_1', snapshot: ID };
	mocks.listRepositories.mockResolvedValue([repository]);
	mocks.listAgents.mockResolvedValue([agent, peer]);
	mocks.listProfiles.mockResolvedValue([]);
	mocks.getSnapshots.mockResolvedValue(
		listing('done', [snapshot('a1b2c3d4', 'nas-backup'), snapshot('b2c3d4e5', 'ws-anna')])
	);
	mocks.startLs.mockResolvedValue(
		treePage([
			node('srv', 'dir'),
			node('notes.txt', 'file'),
			node('latest', 'symlink'),
			node('photo.jpg', 'file')
		])
	);
});

// A dialog that stays open at the end of a test leaves the page locked for the next one.
afterEach(() => {
	document.body.style.pointerEvents = '';
});

async function openDialog(user: ReturnType<typeof userEvent.setup>) {
	await user.click(await screen.findByRole('button', { name: 'Restore…' }));
	return screen.findByRole('dialog');
}

describe('Snapshot page', () => {
	it('fr_rs_02_shows_the_snapshot_header_and_a_back_link_with_the_filters', async () => {
		render(Snapshot);
		expect(await screen.findByRole('heading', { name: 'Snapshot a1b2c3d4' })).toBeVisible();
		expect(screen.getByText(ID)).toBeVisible();
		expect(screen.getByRole('link', { name: /Back to the snapshots/ })).toHaveAttribute(
			'href',
			`/(app)/restore?host=nas-backup&tag=daily&repository=r_1&snapshot=${ID}`
		);
	});

	it('fr_rs_02_says_so_when_the_snapshot_is_not_in_the_stored_list', async () => {
		route.params = { repository: 'r_1', snapshot: 'ff'.repeat(32) };
		render(Snapshot);
		expect(await screen.findByText('The snapshot is not in the stored list')).toBeVisible();
	});

	it('fr_rs_02_loads_the_root_of_a_snapshot_and_a_directory_on_request', async () => {
		const user = userEvent.setup();
		render(Snapshot);
		const tree = await screen.findByRole('tree', { name: 'Files of the snapshot' });
		expect(await within(tree).findByText('notes.txt')).toBeVisible();
		expect(mocks.startLs).toHaveBeenCalledWith(
			'r_1',
			expect.objectContaining({ path: '', offset: 0, limit: 1000 })
		);

		mocks.startLs.mockResolvedValue(treePage([node('data', 'dir', '/srv')], '/srv'));
		await user.click(within(tree).getByRole('button', { name: 'Expand srv' }));
		expect(await within(tree).findByText('data')).toBeVisible();
		expect(mocks.startLs).toHaveBeenLastCalledWith(
			'r_1',
			expect.objectContaining({ path: '/srv' })
		);
	});

	it('fr_rs_02_a_link_shows_its_target_and_no_size_and_cannot_be_expanded', async () => {
		render(Snapshot);
		const tree = await screen.findByRole('tree', { name: 'Files of the snapshot' });
		await within(tree).findByText('latest');
		expect(within(tree).queryByRole('button', { name: 'Expand latest' })).toBeNull();
		expect(within(tree).getByText('→ target-dir')).toBeVisible();
		const item = within(tree).getByText('latest').closest('li');
		expect(item?.textContent).not.toMatch(/\bB\b/);
		// A file keeps its size.
		expect(within(tree).getByText('photo.jpg').closest('li')?.textContent).toMatch(/2(\.0)? KiB/);
	});

	it('fr_rs_02_the_footer_names_the_selected_paths_and_clears_them', async () => {
		const user = userEvent.setup();
		render(Snapshot);
		const tree = await screen.findByRole('tree', { name: 'Files of the snapshot' });
		await user.click(await within(tree).findByRole('checkbox', { name: 'Select notes.txt' }));
		await user.click(within(tree).getByRole('checkbox', { name: 'Select srv' }));
		expect(screen.getByText('Selected: 2')).toBeVisible();
		const list = screen.getByRole('list', { name: 'Selected paths' });
		expect(within(list).getByText('/notes.txt')).toBeVisible();
		expect(within(list).getByText('/srv')).toBeVisible();
		await user.click(screen.getByRole('button', { name: 'Clear selection' }));
		expect(screen.queryByRole('list', { name: 'Selected paths' })).toBeNull();
	});

	it('fr_rs_04_the_target_host_starts_as_the_snapshot_host_when_it_is_active', async () => {
		const user = userEvent.setup();
		mocks.getSnapshots.mockResolvedValue(
			listing('done', [snapshot('a1b2c3d4', 'ws-anna'), snapshot('b2c3d4e5', 'nas-backup')])
		);
		render(Snapshot);
		const dialog = await openDialog(user);
		expect(within(dialog).getByLabelText('Target host')).toHaveValue('a_2');
	});

	it('fr_rs_04_the_target_parameter_preselects_the_target_host_and_the_back_link_keeps_it', async () => {
		const user = userEvent.setup();
		route.url = new URL('http://localhost/restore/r_1/x?target=a_2');
		render(Snapshot);
		expect(await screen.findByRole('link', { name: /Back to the snapshots/ })).toHaveAttribute(
			'href',
			expect.stringContaining('target=a_2')
		);
		const dialog = await openDialog(user);
		expect(within(dialog).getByLabelText('Target host')).toHaveValue('a_2');
	});

	it('fr_rs_04_the_target_parameter_of_an_unknown_host_falls_back_to_the_snapshot_host', async () => {
		const user = userEvent.setup();
		route.url = new URL('http://localhost/restore/r_1/x?target=a_99');
		render(Snapshot);
		const dialog = await openDialog(user);
		expect(within(dialog).getByLabelText('Target host')).toHaveValue(agent.id);
	});

	it('fr_rs_04_offers_the_exclude_field_only_when_nothing_is_selected', async () => {
		const user = userEvent.setup();
		mocks.restoreSnapshot.mockResolvedValue({ id: 'j_8' });
		render(Snapshot);
		const dialog = await openDialog(user);
		const field = within(dialog).getByLabelText('Exclude paths');
		await user.type(field, '*.tmp, /var/cache');
		expect(within(dialog).getByText('Whole snapshot', { selector: 'span' })).toBeVisible();
		await user.click(within(dialog).getByRole('button', { name: 'Start restore' }));
		await waitFor(() =>
			expect(mocks.restoreSnapshot).toHaveBeenCalledWith(
				'r_1',
				expect.objectContaining({ includes: [], excludes: ['*.tmp', '/var/cache'] })
			)
		);
	});

	it('fr_rs_04_shows_no_exclude_field_for_a_selection', async () => {
		const user = userEvent.setup();
		render(Snapshot);
		const tree = await screen.findByRole('tree', { name: 'Files of the snapshot' });
		await user.click(await within(tree).findByRole('checkbox', { name: 'Select notes.txt' }));
		const dialog = await openDialog(user);
		expect(within(dialog).queryByLabelText('Exclude paths')).toBeNull();
		expect(within(dialog).getByText('Selected paths: 1', { selector: 'dd' })).toBeVisible();
	});

	it('fr_rs_04_a_path_outside_the_roots_gets_one_message_and_no_system_warning', async () => {
		const user = userEvent.setup();
		render(Snapshot);
		const dialog = await openDialog(user);
		const field = within(dialog).getByLabelText(/Target path/);
		await user.clear(field);
		await user.type(field, '/etc');
		expect(
			within(dialog).getByText(
				'This path is outside the restore roots. Choose a restore root or a folder inside it.'
			)
		).toBeVisible();
		expect(within(dialog).queryByText('System path')).toBeNull();
		expect(within(dialog).getByRole('button', { name: 'Start restore' })).toBeDisabled();
	});

	it('fr_rs_04_a_restore_root_button_fills_the_target_and_the_rule_is_explained', async () => {
		const user = userEvent.setup();
		render(Snapshot);
		const dialog = await openDialog(user);
		const field = within(dialog).getByLabelText(/Target path/);
		await user.clear(field);
		await user.click(
			within(dialog).getByRole('button', { name: 'Use /var/lib/restichub-restore' })
		);
		expect(field).toHaveValue('/var/lib/restichub-restore/a1b2c3d4');
		expect(
			within(dialog).getByText(
				'A host restores only inside its restore roots. It does not replace the files of the agent itself.'
			)
		).toBeVisible();
	});

	it('fr_rs_04_warns_clearly_when_every_existing_file_is_replaced', async () => {
		const user = userEvent.setup();
		render(Snapshot);
		const dialog = await openDialog(user);
		expect(within(dialog).queryByText('Every existing file in the target is replaced')).toBeNull();
		await user.selectOptions(within(dialog).getByLabelText('Existing files'), 'always');
		expect(within(dialog).getByText('Every existing file in the target is replaced')).toBeVisible();
		await user.click(within(dialog).getByRole('checkbox', { name: /Dry run/ }));
		expect(within(dialog).queryByText('Every existing file in the target is replaced')).toBeNull();
	});

	it('fr_rs_04_the_summary_names_the_snapshot_the_target_and_the_landing_path', async () => {
		const user = userEvent.setup();
		render(Snapshot);
		const tree = await screen.findByRole('tree', { name: 'Files of the snapshot' });
		await user.click(await within(tree).findByRole('checkbox', { name: 'Select notes.txt' }));
		const dialog = await openDialog(user);
		const summary = within(dialog).getByRole('region', { name: 'Summary' });
		expect(summary).toHaveTextContent('a1b2c3d4 · nas-backup · offsite-s3');
		expect(summary).toHaveTextContent('nas-backup: /var/lib/restichub-restore/a1b2c3d4');
		expect(summary).toHaveTextContent('/var/lib/restichub-restore/a1b2c3d4/notes.txt');
		expect(summary).toHaveTextContent('Keep every existing file');
	});

	it('fr_rs_04_starts_a_restore_with_the_selected_paths', async () => {
		const user = userEvent.setup();
		mocks.restoreSnapshot.mockResolvedValue({ id: 'j_9' });
		render(Snapshot);
		const tree = await screen.findByRole('tree', { name: 'Files of the snapshot' });
		await user.click(await within(tree).findByRole('checkbox', { name: 'Select notes.txt' }));
		const dialog = await openDialog(user);
		const field = within(dialog).getByLabelText(/Target path/);
		expect(field).toHaveValue('/var/lib/restichub-restore/a1b2c3d4');
		await user.clear(field);
		await user.type(field, '/var/lib/restichub-restore/out');
		await user.click(within(dialog).getByRole('checkbox', { name: /Dry run/ }));
		expect(within(dialog).getByRole('checkbox', { name: /Verify after/ })).toBeDisabled();
		await user.click(within(dialog).getByRole('button', { name: 'Start restore' }));

		await waitFor(() =>
			expect(mocks.restoreSnapshot).toHaveBeenCalledWith('r_1', {
				agent_id: 'a_1',
				snapshot_id: ID,
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
		render(Snapshot);
		const dialog = await openDialog(user);
		const field = within(dialog).getByLabelText(/Target path/);
		await user.clear(field);
		await user.type(field, 'restore');
		expect(within(dialog).getByText('Enter an absolute path.')).toBeVisible();
		expect(within(dialog).getByRole('button', { name: 'Start restore' })).toBeDisabled();
	});

	it('ac_au_02_a_viewer_gets_no_snapshot_page', async () => {
		session.role = 'viewer';
		render(Snapshot);
		expect(await screen.findByText('Restore needs the Operator role or higher.')).toBeVisible();
		expect(mocks.startLs).not.toHaveBeenCalled();
	});
});
