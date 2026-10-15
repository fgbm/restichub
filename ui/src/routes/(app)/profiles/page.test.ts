/*
 * The Profiles screen edits the scripts of a profile in the form: an Admin changes them, an
 * Operator reads them, and the saved request carries the scripts in their order (FR-HK-09,
 * FR-HK-12).
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Profiles from './+page.svelte';

const mocks = vi.hoisted(() => ({
	listProfiles: vi.fn(),
	listRepositories: vi.fn(),
	listAgents: vi.fn(),
	listPlans: vi.fn(),
	listScriptTemplates: vi.fn(),
	listJobTemplates: vi.fn(),
	listSecrets: vi.fn(),
	createProfile: vi.fn(),
	updateProfile: vi.fn(),
	pushToast: vi.fn()
}));

const session = vi.hoisted(() => ({ role: 'admin' as string }));

vi.mock('#lib/stores/session.svelte.ts', () => ({
	getSession: () => ({
		user: { id: 'u_01', username: 'admin', display_name: 'Anna', role: session.role },
		tenant: { id: 't_01', name: 'acme' },
		role: session.role
	})
}));

vi.mock('#lib/stores/toasts.svelte.ts', () => ({ pushToast: mocks.pushToast }));

vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		service: {
			...actual.service,
			listProfiles: mocks.listProfiles,
			listRepositories: mocks.listRepositories,
			listAgents: mocks.listAgents,
			listPlans: mocks.listPlans,
			listScriptTemplates: mocks.listScriptTemplates,
			listJobTemplates: mocks.listJobTemplates,
			listSecrets: mocks.listSecrets,
			createProfile: mocks.createProfile,
			updateProfile: mocks.updateProfile
		}
	};
});

const STORED_SCRIPT = {
	name: 'Dump the database',
	interpreter: 'bash',
	phase: 'before',
	body: 'pg_dump "$PGDATABASE"\n',
	timeout_seconds: 600,
	failure_policy: 'abort_backup',
	env: [{ name: 'PGDATABASE', kind: 'string', value: 'app' }]
};

function profile(overrides: Record<string, unknown> = {}) {
	return {
		id: 'p_1',
		name: 'db-nightly',
		repository_id: 'r_1',
		paths: { all: ['/srv'], windows: [], linux: [], macos: [] },
		excludes: [],
		exclude_files: [],
		tags: [],
		options: {
			use_fs_snapshot: false,
			one_file_system: false,
			exclude_caches: false,
			limit_upload_kib: 0,
			limit_download_kib: 0
		},
		hooks: [],
		scripts: [STORED_SCRIPT],
		retention: null,
		check: null,
		created_at: '2026-01-01T00:00:00Z',
		updated_at: '2026-01-01T00:00:00Z',
		created_by: 'u_01',
		...overrides
	};
}

function agent(overrides: Record<string, unknown> = {}) {
	return {
		id: 'a_1',
		hostname: 'srv-db-01',
		os: 'linux',
		arch: 'amd64',
		agent_version: '0.4.2',
		restic_version: '0.19.0',
		labels: [],
		restore_roots: [],
		hook_allow_list: 'absent',
		hooks: [],
		server_scripts: 'disabled',
		script_interpreters: [],
		certificate_serial: '01',
		certificate_expires_at: '2999-01-01T00:00:00Z',
		status: 'active',
		last_seen_at: null,
		created_at: '2026-01-01T00:00:00Z',
		revoked_at: null,
		...overrides
	};
}

beforeEach(() => {
	for (const mock of Object.values(mocks)) mock.mockReset();
	session.role = 'admin';
	mocks.listProfiles.mockResolvedValue([profile()]);
	mocks.listRepositories.mockResolvedValue([{ id: 'r_1', name: 'offsite' }]);
	mocks.listAgents.mockResolvedValue([agent()]);
	mocks.listPlans.mockResolvedValue([{ id: 'pl_1', profile_id: 'p_1', agent_id: 'a_1' }]);
	mocks.listScriptTemplates.mockResolvedValue([]);
	mocks.listJobTemplates.mockResolvedValue([]);
	mocks.listSecrets.mockResolvedValue([]);
	const saved = { profile: profile(), warnings: [], plan_warnings: [] };
	mocks.createProfile.mockResolvedValue(saved);
	mocks.updateProfile.mockResolvedValue(saved);
});

async function openEdit(user: ReturnType<typeof userEvent.setup>) {
	await user.click(await screen.findByRole('button', { name: 'More actions for db-nightly' }));
	await user.click(await screen.findByText('Edit'));
	return screen.findByRole('dialog');
}

describe('Profiles screen scripts', () => {
	it('shows the scripts of the profile and warns about the host of its plan', async () => {
		const user = userEvent.setup();
		render(Profiles);
		const dialog = await openEdit(user);
		const before = within(dialog).getByTestId('scripts-before');
		expect(within(before).getByText('Dump the database')).toBeInTheDocument();
		expect(within(dialog).getByText('A host does not accept scripts')).toBeInTheDocument();
		expect(
			within(dialog).getByText('These hosts did not turn on the trust mode: srv-db-01.')
		).toBeInTheDocument();
		expect(
			within(dialog).getByDisplayValue('sudo restichub-agent trust-server-scripts --enable')
		).toBeInTheDocument();
	});

	it('saves a changed script of the profile as an Admin', async () => {
		const user = userEvent.setup();
		render(Profiles);
		const dialog = await openEdit(user);
		await user.click(within(dialog).getByRole('button', { name: 'Show the script' }));
		const name = within(dialog).getByDisplayValue('Dump the database');
		await user.clear(name);
		await user.type(name, 'Dump app');
		await user.click(within(dialog).getByRole('button', { name: 'Save' }));
		await waitFor(() => expect(mocks.updateProfile).toHaveBeenCalledTimes(1));
		const [id, request] = mocks.updateProfile.mock.calls[0];
		expect(id).toBe('p_1');
		expect(request.scripts).toEqual([{ ...STORED_SCRIPT, name: 'Dump app' }]);
	});

	it('shows the scripts to an Operator and sends them back unchanged', async () => {
		session.role = 'operator';
		const user = userEvent.setup();
		render(Profiles);
		const dialog = await openEdit(user);
		expect(
			within(dialog).getByText('Only an Admin changes the scripts of a profile.')
		).toBeInTheDocument();
		expect(within(dialog).queryByRole('button', { name: 'Add script' })).not.toBeInTheDocument();
		await user.click(within(dialog).getByRole('button', { name: 'Save' }));
		await waitFor(() => expect(mocks.updateProfile).toHaveBeenCalledTimes(1));
		expect(mocks.updateProfile.mock.calls[0][1].scripts).toEqual([STORED_SCRIPT]);
	});

	it('creates a profile with a script added from the list', async () => {
		const user = userEvent.setup();
		mocks.listProfiles.mockResolvedValue([]);
		render(Profiles);
		await user.click(await screen.findByRole('button', { name: 'Add the first profile' }));
		const dialog = await screen.findByRole('dialog');
		await user.type(within(dialog).getByLabelText(/^Profile/), 'new');
		await user.type(within(dialog).getByLabelText(/^All systems/), '/srv');
		const before = within(dialog).getByTestId('scripts-before');
		await user.click(within(before).getByRole('button', { name: 'Blank script' }));
		await user.type(within(dialog).getByLabelText(/^Name/), 'Check');
		await user.type(within(dialog).getByLabelText(/^Script\s*\*?\s*$/), 'true');
		await user.click(within(dialog).getByRole('button', { name: 'Save' }));
		await waitFor(() => expect(mocks.createProfile).toHaveBeenCalledTimes(1));
		const request = mocks.createProfile.mock.calls[0][0];
		expect(request.scripts).toMatchObject([
			{ name: 'Check', phase: 'before', body: 'true', interpreter: 'bash' }
		]);
	});
});
