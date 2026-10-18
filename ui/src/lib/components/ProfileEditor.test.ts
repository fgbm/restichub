/*
 * The profile editor page: the scripts of a profile (an Admin changes them, an Operator reads
 * them, FR-HK-09, FR-HK-12), the section summaries, the retention preview, the tags and the run
 * limits (FR-PF-01, ADR-0039).
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '#lib/api/index.ts';
import ProfileEditor from './ProfileEditor.svelte';

const mocks = vi.hoisted(() => ({
	listProfiles: vi.fn(),
	listRepositories: vi.fn(),
	listAgents: vi.fn(),
	listPlans: vi.fn(),
	getJobLimitDefaults: vi.fn(),
	listScriptTemplates: vi.fn(),
	listJobTemplates: vi.fn(),
	listSecrets: vi.fn(),
	createProfile: vi.fn(),
	updateProfile: vi.fn(),
	previewProfile: vi.fn(),
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
			getJobLimitDefaults: mocks.getJobLimitDefaults,
			listScriptTemplates: mocks.listScriptTemplates,
			listJobTemplates: mocks.listJobTemplates,
			listSecrets: mocks.listSecrets,
			createProfile: mocks.createProfile,
			updateProfile: mocks.updateProfile,
			previewProfile: mocks.previewProfile
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
		online: true,
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
	mocks.getJobLimitDefaults.mockResolvedValue({
		stall_timeout_minutes: 45,
		max_duration_minutes: 1080
	});
	mocks.listRepositories.mockResolvedValue([
		{
			id: 'r_1',
			name: 'offsite',
			backend: { kind: 'rest', endpoint: 'https://rest.example.net/a' },
			maintenance_agent_id: null,
			initialized: true
		}
	]);
	mocks.listAgents.mockResolvedValue([agent()]);
	mocks.listPlans.mockResolvedValue([{ id: 'pl_1', profile_id: 'p_1', agent_id: 'a_1' }]);
	mocks.listScriptTemplates.mockResolvedValue([]);
	mocks.listJobTemplates.mockResolvedValue([]);
	mocks.listSecrets.mockResolvedValue([]);
	const saved = { profile: profile(), warnings: [], plan_warnings: [] };
	mocks.createProfile.mockResolvedValue(saved);
	mocks.updateProfile.mockResolvedValue(saved);
	mocks.previewProfile.mockResolvedValue({
		repository_id: 'r_1',
		steps: [
			{
				kind: 'backup',
				maintenance: false,
				command: 'restic -r rest:https://rest.example.net/a backup -- /srv /etc',
				environment: [{ name: 'RESTIC_PASSWORD', value: '***' }]
			},
			{
				kind: 'forget',
				maintenance: true,
				command: 'restic -r rest:https://rest.example.net/a forget --keep-daily=7',
				environment: []
			}
		]
	});
});

function user() {
	return userEvent.setup();
}

async function expand(u: ReturnType<typeof userEvent.setup>, title: RegExp) {
	await u.click(await screen.findByRole('button', { name: title }));
}

describe('Profile editor copy', () => {
	it('starts a new profile as a copy of another profile', async () => {
		render(ProfileEditor, { copyFrom: 'p_1' });
		await screen.findByRole('heading', { level: 1, name: 'New profile' });
		expect(document.querySelector<HTMLInputElement>('#profile-name')?.value).toBe(
			'db-nightly (copy)'
		);
	});
});

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
		await u.type(screen.getByLabelText(/^Profile/), '-2');
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
		expect(screen.getByRole('button', { name: /Run limits.*45m \(default\)/ })).toBeInTheDocument();
	});

	it('folds the exclude list file until it holds a value', async () => {
		render(ProfileEditor, { profileId: 'p_1' });
		await expand(user(), /^Exclusions/);
		const more = await screen.findByRole('button', { name: 'More: a list file on the host' });
		expect(more).toHaveAttribute('aria-expanded', 'false');
		expect(screen.queryByLabelText('Exclude list file on the host')).not.toBeInTheDocument();
		expect(screen.queryByText(/Patterns such as/)).not.toBeInTheDocument();
		expect(screen.getByLabelText('Patterns')).toBeInTheDocument();
	});

	it('opens the exclude list file when the profile already has one', async () => {
		mocks.listProfiles.mockResolvedValue([profile({ exclude_files: ['/etc/excludes.txt'] })]);
		render(ProfileEditor, { profileId: 'p_1' });
		await expand(user(), /^Exclusions/);
		const more = await screen.findByRole('button', { name: 'More: a list file on the host' });
		await waitFor(() => expect(more).toHaveAttribute('aria-expanded', 'true'));
		expect(screen.getByLabelText('Exclude list file on the host')).toHaveValue('/etc/excludes.txt');
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

	it('fr_pf_04_the_commands_come_from_the_server_for_the_draft', async () => {
		const u = user();
		mocks.listProfiles.mockResolvedValue([
			profile({ paths: { all: ['/srv'], windows: ['C:\\Data'], linux: ['/etc'], macos: [] } })
		]);
		mocks.listAgents.mockResolvedValue([
			agent(),
			agent({ id: 'a_2', hostname: 'win-fs01', os: 'windows' })
		]);
		render(ProfileEditor, { profileId: 'p_1' });
		await screen.findByLabelText(/^Profile/);
		expect(screen.queryByText(/^restic -r/)).not.toBeInTheDocument();
		await u.click(screen.getByRole('button', { name: 'Restic commands' }));
		const panel = await screen.findByRole('dialog', { name: 'Restic commands' });
		await waitFor(() => expect(mocks.previewProfile).toHaveBeenCalledTimes(1));
		expect(mocks.previewProfile.mock.calls[0][0]).toMatchObject({
			agent_id: 'a_1',
			profile_id: 'p_1',
			repository_id: 'r_1',
			name: 'db-nightly',
			paths: { all: ['/srv'], windows: ['C:\\Data'], linux: ['/etc'], macos: [] }
		});
		expect(within(panel).getAllByText(/^restic -r/)[0]).toHaveTextContent('/etc');
		expect(within(panel).getByText('Maintenance')).toBeInTheDocument();
		expect(within(panel).getByText('RESTIC_PASSWORD')).toBeInTheDocument();
		expect(within(panel).getByRole('button', { name: 'Copy' })).toBeInTheDocument();
		await u.click(within(panel).getByRole('button', { name: 'Close' }));
	});

	it('fr_pf_04_the_commands_wait_for_a_pause_after_a_change', async () => {
		const u = user();
		mocks.listAgents.mockResolvedValue([
			agent(),
			agent({ id: 'a_2', hostname: 'win-fs01', os: 'windows' })
		]);
		render(ProfileEditor, { profileId: 'p_1' });
		await screen.findByLabelText(/^Profile/);
		await u.click(screen.getByRole('button', { name: 'Restic commands' }));
		const panel = await screen.findByRole('dialog', { name: 'Restic commands' });
		await waitFor(() => expect(mocks.previewProfile).toHaveBeenCalledTimes(1));
		await u.click(within(panel).getByRole('combobox', { name: /^Host/ }));
		await u.click(await screen.findByText('win-fs01', { selector: '.rh-record__option-label' }));
		expect(mocks.previewProfile).toHaveBeenCalledTimes(1);
		await waitFor(() => expect(mocks.previewProfile).toHaveBeenCalledTimes(2), { timeout: 2000 });
		expect(mocks.previewProfile.mock.calls[1][0].agent_id).toBe('a_2');
		await u.click(within(panel).getByRole('button', { name: 'Close' }));
	});

	it('fr_pf_04_a_new_profile_previews_the_unsaved_draft', async () => {
		const u = user();
		render(ProfileEditor);
		await screen.findByLabelText(/^Profile/);
		await u.type(screen.getByLabelText(/^Profile/), 'draft-profile');
		await u.type(screen.getByLabelText(/^All systems/), '/srv');
		await u.click(screen.getByRole('button', { name: 'Restic commands' }));
		await waitFor(() => expect(mocks.previewProfile).toHaveBeenCalledTimes(1));
		const request = mocks.previewProfile.mock.calls[0][0];
		expect(request.name).toBe('draft-profile');
		expect(request.profile_id).toBeUndefined();
		expect(request.paths.all).toEqual(['/srv']);
	});

	it('fr_pf_04_shows_the_field_of_a_refused_draft', async () => {
		const u = user();
		mocks.previewProfile.mockRejectedValue(
			new ApiError({
				status: 400,
				code: 'invalid_argument',
				title: 'Bad Request',
				field: 'paths',
				reason: 'empty'
			})
		);
		render(ProfileEditor, { profileId: 'p_1' });
		await waitFor(() =>
			expect(screen.getByRole('button', { name: 'Restic commands' })).toBeEnabled()
		);
		await u.click(screen.getByRole('button', { name: 'Restic commands' }));
		expect(await screen.findByText('paths · This field is required.')).toBeInTheDocument();
		const panel = screen.getByRole('dialog', { name: 'Restic commands' });
		await u.click(within(panel).getByRole('button', { name: 'Close' }));
	});

	it('ac_ui_11_the_commands_start_at_the_host_of_the_first_plan', async () => {
		const u = user();
		mocks.listAgents.mockResolvedValue([
			agent({ online: false }),
			agent({ id: 'a_2', hostname: 'win-fs01', os: 'windows', online: true })
		]);
		mocks.listPlans.mockResolvedValue([{ id: 'pl_1', profile_id: 'p_1', agent_id: 'a_2' }]);
		render(ProfileEditor, { profileId: 'p_1' });
		await screen.findByLabelText(/^Profile/);
		await u.click(screen.getByRole('button', { name: 'Restic commands' }));
		await waitFor(() => expect(mocks.previewProfile).toHaveBeenCalledTimes(1));
		expect(mocks.previewProfile.mock.calls[0][0].agent_id).toBe('a_2');
		expect(screen.getByRole('combobox', { name: /^Host/ })).toHaveTextContent('win-fs01');
		await u.click(
			within(screen.getByRole('dialog', { name: 'Restic commands' })).getByRole('button', {
				name: 'Close'
			})
		);
	});

	it('ac_ui_11_a_new_profile_starts_at_the_first_online_host', async () => {
		const u = user();
		mocks.listAgents.mockResolvedValue([
			agent({ online: false }),
			agent({ id: 'a_2', hostname: 'win-fs01', os: 'windows', online: true })
		]);
		mocks.listPlans.mockResolvedValue([]);
		render(ProfileEditor);
		await screen.findByLabelText(/^Profile/);
		await u.type(screen.getByLabelText(/^Profile/), 'draft-profile');
		await u.type(screen.getByLabelText(/^All systems/), '/srv');
		await u.click(screen.getByRole('button', { name: 'Restic commands' }));
		await waitFor(() => expect(mocks.previewProfile).toHaveBeenCalledTimes(1));
		expect(mocks.previewProfile.mock.calls[0][0].agent_id).toBe('a_2');
		await u.click(
			within(screen.getByRole('dialog', { name: 'Restic commands' })).getByRole('button', {
				name: 'Close'
			})
		);
	});

	it('ac_ui_11_restic_commands_keep_the_host', async () => {
		const u = user();
		mocks.listAgents.mockResolvedValue([
			agent(),
			agent({ id: 'a_2', hostname: 'win-fs01', os: 'windows' })
		]);
		const first = render(ProfileEditor, { profileId: 'p_1' });
		await screen.findByLabelText(/^Profile/);
		await u.click(screen.getByRole('button', { name: 'Restic commands' }));
		const panel = await screen.findByRole('dialog', { name: 'Restic commands' });
		await u.click(within(panel).getByRole('combobox', { name: /^Host/ }));
		await u.click(await screen.findByText('win-fs01', { selector: '.rh-record__option-label' }));
		expect(JSON.parse(localStorage.getItem('restichub.v1.commands.host') ?? '""')).toBe('a_2');
		first.unmount();

		render(ProfileEditor, { profileId: 'p_1' });
		await screen.findByLabelText(/^Profile/);
		await u.click(screen.getByRole('button', { name: 'Restic commands' }));
		expect(await screen.findByRole('combobox', { name: /^Host/ })).toHaveTextContent('win-fs01');
	});

	it('adr_0047 has no repository field and no repository in the summary', async () => {
		render(ProfileEditor, { profileId: 'p_1' });
		await screen.findByLabelText(/^Profile/);

		expect(document.querySelector('#profile-repository')).toBeNull();
		expect(screen.queryByLabelText('Repository')).not.toBeInTheDocument();
		const side = document.querySelector('.record__side') as HTMLElement;
		expect(within(side).queryByText('Repository')).not.toBeInTheDocument();
		expect(within(side).getByText('db-nightly')).toBeInTheDocument();
	});

	it('ac_ui_10_profile_editor_keeps_the_closed_sections', async () => {
		const u = user();
		mocks.listProfiles.mockResolvedValue([profile()]);
		const first = render(ProfileEditor, { profileId: 'p_1' });
		await screen.findByLabelText(/^Profile/);
		await u.click(screen.getByRole('button', { name: 'Collapse all' }));
		first.unmount();

		render(ProfileEditor, { profileId: 'p_1' });
		const expand = await screen.findByRole('button', { name: 'Expand all' });
		await waitFor(() => expect(screen.queryByLabelText(/^Profile/)).not.toBeInTheDocument());
		await u.click(expand);
		expect(await screen.findByLabelText(/^Profile/)).toBeInTheDocument();
	});

	it('disables the restic commands until the profile has paths and says why', async () => {
		render(ProfileEditor);
		await screen.findByLabelText(/^Profile/);
		const action = screen.getByRole('button', { name: 'Restic commands' });
		expect(action).toBeDisabled();
		expect(action).toHaveAttribute('title', 'Set the paths to see the commands.');
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

	it('keeps the snapshot filter under a borderless disclosure', async () => {
		const u = user();
		render(ProfileEditor);
		await screen.findByLabelText(/^Profile/);
		await expand(u, /^Retention/);
		const toggle = screen.getByRole('button', { name: /Limit the rules to some snapshots/ });
		expect(toggle).toHaveAttribute('aria-expanded', 'false');
		expect(document.querySelector('details')).toBeNull();
		await u.click(toggle);
		expect(screen.getByLabelText('Paths')).toBeInTheDocument();
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
		expect(screen.getByRole('button', { name: 'Remove web' })).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Remove nightly' })).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Remove db' })).toBeInTheDocument();
	});

	it('omits the empty limit fields on an update', async () => {
		const u = user();
		render(ProfileEditor, { profileId: 'p_1' });
		await screen.findByLabelText(/^Profile/);
		await expand(u, /^Run limits/);
		expect(screen.getByLabelText(/^Stop after no progress/)).toHaveAttribute(
			'placeholder',
			'45 (default)'
		);
		await u.type(screen.getByLabelText(/^Profile/), '-2');
		await u.click(screen.getByRole('button', { name: 'Save' }));
		await waitFor(() => expect(mocks.updateProfile).toHaveBeenCalledTimes(1));
		const options = mocks.updateProfile.mock.calls[0][1].options;
		expect(options.stall_timeout_minutes).toBeUndefined();
		expect(options.max_duration_minutes).toBeUndefined();
	});

	it('starts a new profile with the server defaults and the standard retention', async () => {
		const u = user();
		render(ProfileEditor);
		await screen.findByLabelText(/^Profile/);
		await expand(u, /^Run limits/);
		expect(screen.getByLabelText(/^Stop after no progress/)).toHaveValue(45);
		expect(screen.getByLabelText(/^Maximum duration/)).toHaveValue(18);
		expect(screen.getByRole('button', { name: /Retention.*last 3/ })).toBeInTheDocument();
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
