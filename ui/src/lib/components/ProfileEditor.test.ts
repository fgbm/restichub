/*
 * The profile editor page: the scripts of a profile (an Admin changes them, an Operator reads
 * them, FR-HK-09, FR-HK-12), the section summaries, the retention preview, the tags and the run
 * limits (FR-PF-01, ADR-0039).
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ProfileEditor from './ProfileEditor.svelte';

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
	pushToast: vi.fn(),
	goto: vi.fn()
}));

const session = vi.hoisted(() => ({ role: 'admin' as string }));

vi.mock('$app/paths', () => ({
	resolve: (path: string, params?: Record<string, string>) =>
		params ? path.replace('[id]', params.id) : path
}));
vi.mock('$app/navigation', () => ({ goto: mocks.goto }));
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

function user() {
	return userEvent.setup();
}

async function expand(u: ReturnType<typeof userEvent.setup>, title: RegExp) {
	await u.click(await screen.findByRole('button', { name: title }));
}

describe('Profile editor scripts', () => {
	it('shows the scripts of the profile and warns about the host of its plan', async () => {
		const u = user();
		render(ProfileEditor, { profileId: 'p_1' });
		await expand(u, /^Scripts on the host/);
		const before = screen.getByTestId('scripts-before');
		expect(within(before).getByText('Dump the database')).toBeInTheDocument();
		expect(screen.getByText('A host does not accept scripts')).toBeInTheDocument();
		expect(
			screen.getByText('These hosts did not turn on the trust mode: srv-db-01.')
		).toBeInTheDocument();
	});

	it('saves a changed script of the profile as an Admin', async () => {
		const u = user();
		render(ProfileEditor, { profileId: 'p_1' });
		await expand(u, /^Scripts on the host/);
		await u.click(screen.getByRole('button', { name: 'Show the script' }));
		const name = screen.getByDisplayValue('Dump the database');
		await u.clear(name);
		await u.type(name, 'Dump app');
		await u.click(screen.getByRole('button', { name: 'Save' }));
		await waitFor(() => expect(mocks.updateProfile).toHaveBeenCalledTimes(1));
		const [id, request] = mocks.updateProfile.mock.calls[0];
		expect(id).toBe('p_1');
		expect(request.scripts).toEqual([{ ...STORED_SCRIPT, name: 'Dump app' }]);
		await waitFor(() => expect(mocks.goto).toHaveBeenCalledWith('/(app)/profiles'));
	});

	it('shows the scripts to an Operator and sends them back unchanged', async () => {
		session.role = 'operator';
		const u = user();
		render(ProfileEditor, { profileId: 'p_1' });
		await expand(u, /^Scripts on the host/);
		expect(screen.getByText('Only an Admin changes the scripts of a profile.')).toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Use a job template' })).not.toBeInTheDocument();
		await u.click(screen.getByRole('button', { name: 'Save' }));
		await waitFor(() => expect(mocks.updateProfile).toHaveBeenCalledTimes(1));
		expect(mocks.updateProfile.mock.calls[0][1].scripts).toEqual([STORED_SCRIPT]);
	});

	it('creates a profile with a script added in the editor', async () => {
		const u = user();
		render(ProfileEditor);
		await u.type(await screen.findByLabelText(/^Profile/), 'new');
		await u.type(screen.getByLabelText(/^All systems/), '/srv');
		await u.click(
			screen.getByRole('switch', { name: 'Turn on the section "Scripts on the host"' })
		);
		const before = screen.getByTestId('scripts-before');
		await u.click(within(before).getByRole('button', { name: 'Blank script' }));
		await u.type(screen.getByLabelText(/^Name/), 'Check');
		await u.type(screen.getByLabelText(/^Script\s*\*?\s*$/), 'true');
		await u.click(screen.getByRole('button', { name: 'Save' }));
		await waitFor(() => expect(mocks.createProfile).toHaveBeenCalledTimes(1));
		const request = mocks.createProfile.mock.calls[0][0];
		expect(request.scripts).toMatchObject([
			{ name: 'Check', phase: 'before', body: 'true', interpreter: 'bash' }
		]);
	});
});

describe('Profile editor sections', () => {
	it('shows a one-line summary on each collapsed section', async () => {
		mocks.listProfiles.mockResolvedValue([
			profile({
				tags: ['web'],
				retention: { keep_last: 3, keep_daily: 7, keep_tags: ['keep'] },
				check: { read_data_subset: '5%' }
			})
		]);
		render(ProfileEditor, { profileId: 'p_1' });
		expect(
			await screen.findByRole('button', { name: /Retention.*last 3 · by day 7 · tags: keep/ })
		).toBeInTheDocument();
		expect(
			screen.getByRole('button', { name: /Repository check.*read 5% of the data/ })
		).toBeInTheDocument();
		expect(screen.getByRole('button', { name: /Tags.*web/ })).toBeInTheDocument();
		expect(screen.getByRole('button', { name: /Run limits.*default/ })).toBeInTheDocument();
	});

	it('has no hooks section', async () => {
		render(ProfileEditor, { profileId: 'p_1' });
		await screen.findByLabelText(/^Profile/);
		expect(screen.queryByText('Hooks')).not.toBeInTheDocument();
	});

	it('turns a section on with its switch and opens it', async () => {
		const u = user();
		render(ProfileEditor);
		await screen.findByLabelText(/^Profile/);
		const toggle = screen.getByRole('button', { name: /^Tags/ });
		expect(toggle).toHaveAttribute('aria-expanded', 'false');
		await u.click(screen.getByRole('switch', { name: 'Turn on the section "Tags"' }));
		expect(toggle).toHaveAttribute('aria-expanded', 'true');
	});

	it('shows the commands of the host system in the side panel', async () => {
		const u = user();
		mocks.listProfiles.mockResolvedValue([
			profile({ paths: { all: ['/srv'], windows: ['C:\\Data'], linux: ['/etc'], macos: [] } })
		]);
		render(ProfileEditor, { profileId: 'p_1' });
		await screen.findByLabelText(/^Profile/);
		expect(screen.getByText(/^restic backup/)).toHaveTextContent('/etc');
		await u.selectOptions(screen.getByLabelText('Commands for the host:'), 'windows');
		expect(screen.getByText(/^restic backup/)).toHaveTextContent('C:\\Data');
		expect(screen.getByText(/^restic backup/)).not.toHaveTextContent('/etc');
	});
});

describe('Profile editor retention', () => {
	it('applies a preset and shows what stays', async () => {
		const u = user();
		render(ProfileEditor);
		await screen.findByLabelText(/^Profile/);
		await expand(u, /^Retention/);
		await u.click(screen.getByRole('button', { name: 'Minimum' }));
		expect(screen.getByLabelText('By day')).toHaveValue(7);
		expect(screen.getByLabelText('By week')).toHaveValue(0);
		expect(screen.getByTestId('keep-line')).toHaveTextContent(/keep about \d+ snapshots of 63/);
	});

	it('says plainly that no rule means no deletion', async () => {
		const u = user();
		render(ProfileEditor);
		await screen.findByLabelText(/^Profile/);
		await expand(u, /^Retention/);
		for (const label of ['Latest snapshots', 'By day', 'By week', 'By month']) {
			await u.clear(screen.getByLabelText(label));
		}
		expect(screen.getByTestId('keep-line')).toHaveTextContent('nothing is deleted');
		expect(screen.getByText('No rules: nothing is deleted')).toBeInTheDocument();
	});

	it('shows an error for a period that does not parse', async () => {
		const u = user();
		render(ProfileEditor);
		await screen.findByLabelText(/^Profile/);
		await expand(u, /^Retention/);
		await u.type(screen.getByLabelText('All snapshots in a period'), '3 weeks');
		expect(screen.getByText(/The period is not valid/)).toBeInTheDocument();
	});
});

describe('Profile editor tags and limits', () => {
	it('turns tags typed with commas and spaces into chips', async () => {
		const u = user();
		render(ProfileEditor);
		await screen.findByLabelText(/^Profile/);
		await u.click(screen.getByRole('switch', { name: 'Turn on the section "Tags"' }));
		await u.type(document.getElementById('profile-tags') as HTMLElement, 'web, nightly db ');
		expect(screen.getByRole('button', { name: 'Remove the tag web' })).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Remove the tag nightly' })).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Remove the tag db' })).toBeInTheDocument();
	});

	it('sends the default limits when the fields are empty', async () => {
		const u = user();
		render(ProfileEditor, { profileId: 'p_1' });
		await screen.findByLabelText(/^Profile/);
		await u.click(screen.getByRole('button', { name: 'Save' }));
		await waitFor(() => expect(mocks.updateProfile).toHaveBeenCalledTimes(1));
		const options = mocks.updateProfile.mock.calls[0][1].options;
		expect(options.stall_timeout_minutes).toBe(60);
		expect(options.max_duration_minutes).toBe(48 * 60);
	});

	it('sends the limits that the operator enters', async () => {
		const u = user();
		render(ProfileEditor, { profileId: 'p_1' });
		await screen.findByLabelText(/^Profile/);
		await expand(u, /^Run limits/);
		await u.type(screen.getByLabelText(/^Stop after no progress/), '90');
		await u.type(screen.getByLabelText(/^Maximum duration/), '2');
		await u.click(screen.getByRole('button', { name: 'Save' }));
		await waitFor(() => expect(mocks.updateProfile).toHaveBeenCalledTimes(1));
		const options = mocks.updateProfile.mock.calls[0][1].options;
		expect(options.stall_timeout_minutes).toBe(90);
		expect(options.max_duration_minutes).toBe(120);
	});

	it('shows the stored limits when the profile is edited', async () => {
		const u = user();
		mocks.listProfiles.mockResolvedValue([
			profile({
				options: {
					use_fs_snapshot: false,
					one_file_system: false,
					exclude_caches: false,
					limit_upload_kib: 0,
					limit_download_kib: 0,
					stall_timeout_minutes: 30,
					max_duration_minutes: 600
				}
			})
		]);
		render(ProfileEditor, { profileId: 'p_1' });
		await expand(u, /^Run limits/);
		expect(screen.getByLabelText(/^Stop after no progress/)).toHaveValue(30);
		expect(screen.getByLabelText(/^Maximum duration/)).toHaveValue(10);
	});
});
