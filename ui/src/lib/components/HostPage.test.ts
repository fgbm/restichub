/*
 * The host page shows one host and its state, and opens the small dialogs of the host actions
 * (FR-ID-01, ADR-0035, ADR-0037). The test replaces the session, the toast store and the
 * service, so it renders the page alone.
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '#lib/api/index.ts';
import HostPage from './HostPage.svelte';

const mocks = vi.hoisted(() => ({
	getAgent: vi.fn(),
	listUsers: vi.fn(),
	listJobs: vi.fn(),
	listPlans: vi.fn(),
	listAlerts: vi.fn(),
	renameAgent: vi.fn(),
	getAgentParameters: vi.fn(),
	replaceAgentParameters: vi.fn(),
	listSecrets: vi.fn(),
	pushToast: vi.fn()
}));

const session = vi.hoisted(() => ({ role: 'admin' as string }));

vi.mock('$app/paths', () => ({
	resolve: (path: string, params?: Record<string, string>) =>
		params ? path.replace('[id]', params.id) : path
}));
vi.mock('#lib/stores/session.svelte.ts', () => ({
	getSession: () => ({
		user: { id: 'u_01', username: 'admin', display_name: 'Anna', role: session.role },
		tenant: { id: 't_01', name: 'acme' },
		role: session.role
	})
}));
vi.mock('#lib/stores/toasts.svelte.ts', () => ({ pushToast: mocks.pushToast }));

const stream = vi.hoisted(() => ({
	listeners: [] as ((event: { type: string; agent_id?: string }) => void)[]
}));

vi.mock('#lib/stores/events.svelte.ts', () => ({
	isStreamConnected: () => false,
	onEvent: (listener: (event: { type: string; agent_id?: string }) => void) => {
		stream.listeners.push(listener);
		return () => {
			stream.listeners = stream.listeners.filter((item) => item !== listener);
		};
	}
}));

vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		service: {
			...actual.service,
			getAgent: mocks.getAgent,
			listUsers: mocks.listUsers,
			listJobs: mocks.listJobs,
			listPlans: mocks.listPlans,
			listAlerts: mocks.listAlerts,
			renameAgent: mocks.renameAgent,
			getAgentParameters: mocks.getAgentParameters,
			replaceAgentParameters: mocks.replaceAgentParameters,
			listSecrets: mocks.listSecrets
		}
	};
});

const FUTURE = '2999-01-01T00:00:00Z';

function agent(overrides: Record<string, unknown> = {}) {
	return {
		id: 'a_01',
		hostname: 'srv-db-01',
		os: 'linux',
		arch: 'amd64',
		agent_version: '0.4.2',
		restic_version: '0.17.0',
		control_plane_version: '0.4.2',
		version_status: 'current',
		status: 'active',
		online: true,
		labels: ['db', 'prod'],
		restore_roots: ['/var/lib/restichub-restore'],
		certificate_serial: '0f01',
		certificate_expires_at: FUTURE,
		last_certificate_rotation_at: null,
		superseded_certificate_used_at: null,
		protocol_version: '1.1',
		first_connected_at: '2026-10-01T00:00:00Z',
		last_seen_at: '2026-10-16T00:00:00Z',
		created_at: '2026-09-01T00:00:00Z',
		possible_duplicate_of: null,
		server_scripts: 'disabled',
		script_interpreters: [],
		revoked_at: null,
		...overrides
	};
}

describe('the host page', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		session.role = 'admin';
		mocks.getAgent.mockResolvedValue(agent());
		mocks.listUsers.mockResolvedValue([]);
		mocks.listJobs.mockResolvedValue({ items: [], next_cursor: null });
		mocks.listPlans.mockResolvedValue([]);
		mocks.listAlerts.mockResolvedValue({ items: [], next_cursor: null });
		mocks.getAgentParameters.mockResolvedValue({ agent_id: 'a_01', parameters: [] });
		mocks.replaceAgentParameters.mockResolvedValue({ agent_id: 'a_01', parameters: [] });
		mocks.listSecrets.mockResolvedValue([]);
	});

	it('shows the properties, the restore directories and the rule for the targets', async () => {
		render(HostPage, { hostId: 'a_01' });

		expect(await screen.findByRole('heading', { level: 1, name: 'srv-db-01' })).toBeVisible();
		expect(mocks.getAgent).toHaveBeenCalledWith('a_01');
		expect(screen.getAllByText('Online').length).toBeGreaterThan(0);
		expect(screen.getByText('0.17.0')).toBeInTheDocument();
		expect(screen.getByText('prod')).toBeInTheDocument();
		expect(screen.getByDisplayValue('/var/lib/restichub-restore')).toBeInTheDocument();
		expect(screen.getByText(/never replaces the files of the agent itself/)).toBeInTheDocument();
		expect(screen.getByText('Restore directory')).toBeInTheDocument();
		expect(screen.getByText(/only into this directory/)).toBeInTheDocument();
		expect(screen.getByText('Linux')).toBeInTheDocument();
		expect(screen.queryByText('linux')).not.toBeInTheDocument();
		expect(screen.getByRole('link', { name: 'Restore to this host' })).toBeInTheDocument();
		expect(screen.getByDisplayValue('0f01')).toBeInTheDocument();
	});

	it('names several restore directories in the plural', async () => {
		mocks.getAgent.mockResolvedValue(
			agent({ restore_roots: ['/var/lib/restichub-restore', '/mnt/restore'] })
		);
		render(HostPage, { hostId: 'a_01' });
		expect(await screen.findByText('Restore directories')).toBeInTheDocument();
		expect(screen.getByText(/only into these directories/)).toBeInTheDocument();
	});

	it('ac_rs_05 explains a host without restore directories', async () => {
		mocks.getAgent.mockResolvedValue(agent({ restore_roots: [] }));
		render(HostPage, { hostId: 'a_01' });

		expect(await screen.findByText(/reported no restore directory/)).toBeInTheDocument();
		expect(screen.getByText(/update the agent package/)).toBeInTheDocument();
	});

	it('shows an offline host and the open alerts of the host only', async () => {
		mocks.getAgent.mockResolvedValue(agent({ online: false }));
		mocks.listAlerts.mockResolvedValue({
			items: [
				{ id: 'al_1', agent_id: 'a_01' },
				{ id: 'al_2', agent_id: 'a_99' }
			],
			next_cursor: null
		});
		render(HostPage, { hostId: 'a_01' });

		expect((await screen.findAllByText('Offline')).length).toBeGreaterThan(0);
		const state = screen.getByText('Open alerts').closest('.key-value__row') as HTMLElement;
		expect(within(state).getByText('1')).toBeInTheDocument();
	});

	it('fr_ui_02 a host event reads the host again and shows it offline', async () => {
		mocks.getAgent.mockResolvedValueOnce(agent()).mockResolvedValue(agent({ online: false }));
		render(HostPage, { hostId: 'a_01' });

		await screen.findByRole('heading', { level: 1, name: 'srv-db-01' });
		expect(screen.queryByText('Offline')).not.toBeInTheDocument();
		for (const listener of stream.listeners) listener({ type: 'host.offline', agent_id: 'a_01' });

		expect((await screen.findAllByText('Offline')).length).toBeGreaterThan(0);
		expect(screen.queryByText('Online')).not.toBeInTheDocument();
	});

	it('fr_hk_08 shows the trust mode with the interpreters', async () => {
		mocks.getAgent.mockResolvedValue(
			agent({ server_scripts: 'enabled', script_interpreters: ['bash', 'sh'] })
		);
		render(HostPage, { hostId: 'a_01' });

		expect(await screen.findByText('bash, sh')).toBeInTheDocument();
		expect(
			screen.getByText(
				'The control plane can run any code on this host with the rights of the agent.'
			)
		).toBeInTheDocument();
	});

	it('fr_hk_08 shows the command that turns the trust mode on', async () => {
		render(HostPage, { hostId: 'a_01' });

		await screen.findByRole('heading', { level: 1, name: 'srv-db-01' });
		expect(screen.getByDisplayValue(/trust-server-scripts --enable/)).toBeInTheDocument();
	});

	it('shows the recent jobs and the plans of the host with their links', async () => {
		mocks.listJobs.mockResolvedValue({
			items: [
				{
					id: 'j_1',
					kind: 'backup',
					status: 'succeeded',
					plan_name: 'nightly',
					created_at: '2026-10-15T00:00:00Z'
				}
			],
			next_cursor: null
		});
		mocks.listPlans.mockResolvedValue([
			{ id: 'p_1', name: 'nightly', agent_id: 'a_01', cron: '0 2 * * *' },
			{ id: 'p_2', name: 'other', agent_id: 'a_02', cron: '0 3 * * *' }
		]);
		render(HostPage, { hostId: 'a_01' });

		expect(await screen.findByRole('link', { name: 'nightly' })).toHaveAttribute(
			'href',
			'/(app)/jobs/j_1'
		);
		expect(mocks.listJobs).toHaveBeenCalledWith({ agent_id: 'a_01', limit: 5 });
		expect(screen.getByText('0 2 * * *')).toBeInTheDocument();
		expect(screen.queryByText('other')).not.toBeInTheDocument();
		expect(screen.getByRole('link', { name: 'All jobs of this host' })).toHaveAttribute(
			'href',
			'/(app)/jobs?agent_id=a_01'
		);
	});

	it('shows the duplicate banner and opens the replace dialog', async () => {
		const user = userEvent.setup();
		mocks.getAgent.mockResolvedValue(
			agent({
				possible_duplicate_of: {
					id: 'a_00',
					hostname: 'srv-db-old',
					created_at: '2026-01-01T00:00:00Z',
					last_seen_at: '2026-10-10T00:00:00Z',
					is_older: true
				}
			})
		);
		render(HostPage, { hostId: 'a_01' });

		await screen.findByText(/may duplicate the older record/);
		const [banner] = screen.getAllByRole('button', { name: 'Replace the old record' });
		await user.click(banner);
		expect(await screen.findByRole('dialog', { name: 'Replace the old record?' })).toBeVisible();
	});

	it('renames the host in a small dialog and reads the host again', async () => {
		const user = userEvent.setup();
		mocks.renameAgent.mockResolvedValue(agent({ hostname: 'srv-db-02' }));
		render(HostPage, { hostId: 'a_01' });

		await screen.findByRole('heading', { level: 1, name: 'srv-db-01' });
		await user.click(screen.getByRole('button', { name: 'Rename' }));
		const dialog = await screen.findByRole('dialog', { name: 'Rename the host' });
		const field = within(dialog).getByLabelText(/Host name/);
		await user.clear(field);
		await user.type(field, 'srv-db-02');
		await user.click(within(dialog).getByRole('button', { name: 'Rename' }));

		await waitFor(() => expect(mocks.renameAgent).toHaveBeenCalledWith('a_01', 'srv-db-02'));
		await waitFor(() => expect(mocks.getAgent.mock.calls.length).toBeGreaterThan(1));
	});

	it('keeps the re-register and the revoke actions in the dangerous section only', async () => {
		render(HostPage, { hostId: 'a_01' });

		await screen.findByRole('heading', { level: 1, name: 'srv-db-01' });
		expect(screen.queryByRole('button', { name: /More actions/ })).not.toBeInTheDocument();
		const danger = screen
			.getByRole('button', { name: /Dangerous actions/ })
			.closest('section') as HTMLElement;
		expect(within(danger).getByRole('button', { name: 'Re-register' })).toBeInTheDocument();
		expect(within(danger).getByRole('button', { name: 'Revoke agent' })).toBeInTheDocument();
	});

	it('hides the write actions from a viewer', async () => {
		session.role = 'viewer';
		render(HostPage, { hostId: 'a_01' });

		await screen.findByRole('heading', { level: 1, name: 'srv-db-01' });
		expect(screen.queryByRole('button', { name: 'Rename' })).not.toBeInTheDocument();
		expect(screen.queryByText('Dangerous actions')).not.toBeInTheDocument();
	});

	it('shows a revoked host without actions', async () => {
		mocks.getAgent.mockResolvedValue(agent({ status: 'revoked' }));
		render(HostPage, { hostId: 'a_01' });

		expect(
			await screen.findByText('This host is revoked. It does not connect any more.')
		).toBeVisible();
		expect(screen.queryByRole('button', { name: 'Rename' })).not.toBeInTheDocument();
	});

	it('shows the connection of a host from the agent view, not from the dashboard', async () => {
		render(HostPage, { hostId: 'a_01' });

		await screen.findByRole('heading', { level: 1, name: 'srv-db-01' });
		const side = screen.getByRole('complementary');
		const state = within(side).getByText('Connection').closest('.key-value__row') as HTMLElement;
		expect(within(state).getByText('Online')).toBeInTheDocument();
		expect(within(side).queryByText('Restore directories')).not.toBeInTheDocument();
		expect(within(side).queryByText('Last seen')).not.toBeInTheDocument();
	});

	it('links the restore page with the host as the target host', async () => {
		render(HostPage, { hostId: 'a_01' });

		expect(await screen.findByRole('link', { name: 'Restore to this host' })).toHaveAttribute(
			'href',
			'/(app)/restore?target=a_01'
		);
	});

	it('names the user that revoked the host when the user list is available', async () => {
		mocks.listUsers.mockResolvedValue([{ id: 'u_07', display_name: 'Boris' }]);
		mocks.getAgent.mockResolvedValue(
			agent({ status: 'revoked', online: false, revoked_by_user_id: 'u_07' })
		);
		render(HostPage, { hostId: 'a_01' });

		expect(await screen.findByText('Revoked by Boris')).toBeVisible();
	});

	it('does not read the users for a viewer and shows no name', async () => {
		session.role = 'viewer';
		mocks.getAgent.mockResolvedValue(agent({ status: 'revoked', revoked_by_user_id: 'u_07' }));
		render(HostPage, { hostId: 'a_01' });

		await screen.findByText('This host is revoked. It does not connect any more.');
		expect(mocks.listUsers).not.toHaveBeenCalled();
		expect(screen.queryByText(/Revoked by/)).not.toBeInTheDocument();
	});

	it('shows the error of a host that does not exist', async () => {
		mocks.getAgent.mockRejectedValue(new Error('not found'));
		render(HostPage, { hostId: 'missing' });

		expect(await screen.findByRole('alert')).toBeInTheDocument();
	});
});

describe('the parameter values of a host (FR-PF-10 to FR-PF-12, ADR-0047)', () => {
	function row(overrides: Record<string, unknown> = {}) {
		return {
			name: 'pg_dir',
			kind: 'text',
			required: true,
			default_value: null,
			description: 'Directory of the dumps',
			needed_by: ['pg-dump'],
			value: null,
			unused: false,
			hidden: false,
			...overrides
		};
	}

	const SECRET_ROW = row({
		name: 'pg_password',
		kind: 'secret',
		value: { kind: 'secret', value: null, secret_id: 's_1', secret_name: 'db.password' }
	});

	beforeEach(() => {
		vi.clearAllMocks();
		session.role = 'admin';
		mocks.getAgent.mockResolvedValue(agent());
		mocks.listUsers.mockResolvedValue([]);
		mocks.listJobs.mockResolvedValue({ items: [], next_cursor: null });
		mocks.listPlans.mockResolvedValue([]);
		mocks.listAlerts.mockResolvedValue({ items: [], next_cursor: null });
		mocks.getAgentParameters.mockResolvedValue({ agent_id: 'a_01', parameters: [] });
		mocks.replaceAgentParameters.mockResolvedValue({ agent_id: 'a_01', parameters: [] });
		mocks.listSecrets.mockResolvedValue([]);
	});

	// The section is open by default; a remembered state can fold it.
	async function openSection(user: ReturnType<typeof userEvent.setup>): Promise<void> {
		const button = screen.getByRole('button', { name: /^Parameter values/ });
		if (button.getAttribute('aria-expanded') !== 'true') await user.click(button);
	}

	it('ac_pf_06_gives_a_value_and_sends_the_whole_set', async () => {
		const user = userEvent.setup();
		mocks.getAgentParameters.mockResolvedValue({
			agent_id: 'a_01',
			parameters: [row(), SECRET_ROW]
		});
		const saved = {
			agent_id: 'a_01',
			parameters: [
				row({ value: { kind: 'text', value: '/srv/db', secret_id: null, secret_name: null } }),
				SECRET_ROW
			]
		};
		mocks.replaceAgentParameters.mockResolvedValue(saved);
		render(HostPage, { hostId: 'a_01' });
		await screen.findByRole('heading', { level: 1, name: 'srv-db-01' });

		await openSection(user);
		await user.type(screen.getByLabelText('Value of pg_dir'), '/srv/db');
		await user.click(screen.getByRole('button', { name: 'Save the values' }));

		await waitFor(() =>
			expect(mocks.replaceAgentParameters).toHaveBeenCalledWith('a_01', [
				{ name: 'pg_dir', kind: 'text', value: '/srv/db' },
				{ name: 'pg_password', kind: 'secret', secret_id: 's_1' }
			])
		);
		expect(mocks.pushToast).toHaveBeenCalledWith('success', 'The values are saved.');
	});

	it('ac_pf_10_hides_the_text_values_from_a_viewer', async () => {
		session.role = 'viewer';
		mocks.getAgentParameters.mockResolvedValue({
			agent_id: 'a_01',
			parameters: [
				row({
					value: { kind: 'text', value: null, secret_id: null, secret_name: null },
					hidden: true
				}),
				SECRET_ROW
			]
		});
		render(HostPage, { hostId: 'a_01' });
		await screen.findByRole('heading', { level: 1, name: 'srv-db-01' });

		await openSection(userEvent.setup());
		expect(
			screen.getByText('Your role does not show the values of text parameters.')
		).toBeInTheDocument();
		expect(screen.getByDisplayValue('••••••')).toBeDisabled();
		expect(screen.getByDisplayValue('db.password')).toBeDisabled();
		expect(screen.queryByRole('button', { name: 'Save the values' })).not.toBeInTheDocument();
	});

	it('ac_pf_07_shows_the_plans_of_a_refused_change', async () => {
		const user = userEvent.setup();
		mocks.getAgentParameters.mockResolvedValue({ agent_id: 'a_01', parameters: [row()] });
		mocks.replaceAgentParameters.mockRejectedValue(
			new ApiError({
				status: 400,
				code: 'parameter_missing_in_plans',
				title: 'Bad Request',
				field: 'parameters',
				parameters: ['pg_dir'],
				blockers: { items: [{ kind: 'plan', id: 'pl_1', name: 'pg-dump-daily' }], total: 1 }
			})
		);
		render(HostPage, { hostId: 'a_01' });
		await screen.findByRole('heading', { level: 1, name: 'srv-db-01' });

		await openSection(user);
		await user.type(screen.getByLabelText('Value of pg_dir'), '/srv/db');
		await user.click(screen.getByRole('button', { name: 'Save the values' }));

		expect(
			await screen.findByText(
				'The values are not saved: plans would stay without the required parameter pg_dir.'
			)
		).toBeInTheDocument();
		expect(screen.getByRole('link', { name: 'pg-dump-daily' })).toHaveAttribute(
			'href',
			'/(app)/plans/pl_1'
		);
	});

	it('fr_pf_10_removes_a_value_that_no_profile_needs', async () => {
		const user = userEvent.setup();
		const unused = row({
			name: 'old_dir',
			required: false,
			needed_by: [],
			unused: true,
			value: { kind: 'text', value: '/var/backups/old', secret_id: null, secret_name: null }
		});
		mocks.getAgentParameters.mockResolvedValue({
			agent_id: 'a_01',
			parameters: [row(), unused]
		});
		mocks.replaceAgentParameters.mockResolvedValue({ agent_id: 'a_01', parameters: [row()] });
		render(HostPage, { hostId: 'a_01' });
		await screen.findByRole('heading', { level: 1, name: 'srv-db-01' });

		await openSection(user);
		expect(screen.getByText('No profile needs it')).toBeInTheDocument();
		await user.click(screen.getByRole('button', { name: 'Delete the value of old_dir' }));
		expect(screen.queryByDisplayValue('/var/backups/old')).not.toBeInTheDocument();
		await user.click(screen.getByRole('button', { name: 'Save the values' }));
		await waitFor(() => expect(mocks.replaceAgentParameters).toHaveBeenCalledWith('a_01', []));
	});

	it('adr_0047_locks_a_secret_value_for_an_operator', async () => {
		const user = userEvent.setup();
		session.role = 'operator';
		mocks.getAgentParameters.mockResolvedValue({
			agent_id: 'a_01',
			parameters: [row(), SECRET_ROW]
		});
		mocks.listSecrets.mockResolvedValue([
			{
				id: 's_1',
				name: 'db.password',
				purpose: 'hook_secret',
				created_at: '2026-01-01T00:00:00Z',
				updated_at: '2026-01-01T00:00:00Z',
				used_by: []
			}
		]);
		render(HostPage, { hostId: 'a_01' });
		await screen.findByRole('heading', { level: 1, name: 'srv-db-01' });

		await openSection(user);
		expect(screen.getByText('Only an Admin changes a secret value.')).toBeInTheDocument();
		expect(screen.getByDisplayValue('db.password')).toBeDisabled();
	});
});
