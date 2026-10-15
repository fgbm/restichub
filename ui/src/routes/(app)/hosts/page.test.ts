/*
 * The Hosts screen lists the pending registrations and renames a host (FR-ID-01, ADR-0026).
 *
 * The test replaces the session store, the toast store and the service, so it renders the
 * screen alone. The pending list keeps only a token that can still enroll a host, and only
 * an Admin sees a write action.
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Hosts from './+page.svelte';

const mocks = vi.hoisted(() => ({
	listAgents: vi.fn(),
	listEnrollmentTokens: vi.fn(),
	createEnrollmentToken: vi.fn(),
	revokeEnrollmentToken: vi.fn(),
	renameAgent: vi.fn(),
	pushToast: vi.fn()
}));

const session = vi.hoisted(() => ({ role: 'admin' as string }));

vi.mock('#lib/stores/session.svelte.ts', () => ({
	getSession: () => ({
		user: {
			id: 'u_01',
			username: 'admin',
			display_name: 'Anna Petrova',
			role: 'admin',
			disabled: false,
			shared: false
		},
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
			listAgents: mocks.listAgents,
			listEnrollmentTokens: mocks.listEnrollmentTokens,
			createEnrollmentToken: mocks.createEnrollmentToken,
			revokeEnrollmentToken: mocks.revokeEnrollmentToken,
			renameAgent: mocks.renameAgent
		}
	};
});

const PAST = '2026-01-01T00:00:00Z';
const FUTURE = '2999-01-01T00:00:00Z';

function token(overrides: Record<string, unknown> = {}) {
	return {
		id: 'e_1',
		display_prefix: 'ab12cd34',
		labels: [],
		expires_at: FUTURE,
		used_at: null,
		revoked_at: null,
		revoked_by_user_id: null,
		created_at: '2026-10-13T00:00:00Z',
		created_by: 'u_01',
		...overrides
	};
}

function agent(overrides: Record<string, unknown> = {}) {
	return {
		id: 'a_01',
		hostname: 'win-fs01',
		os: 'windows',
		arch: 'amd64',
		agent_version: '0.4.2',
		restic_version: '0.17.0',
		restore_roots: [],
		labels: [],
		certificate_serial: '0f01',
		certificate_expires_at: FUTURE,
		last_certificate_rotation_at: null,
		superseded_certificate_used_at: null,
		protocol_version: '1.1',
		control_plane_version: '0.4.2',
		version_status: 'current',
		status: 'active',
		last_seen_at: PAST,
		created_at: PAST,
		revoked_at: null,
		revoked_by_user_id: null,
		hook_allow_list: 'absent',
		hooks: [],
		server_scripts: 'disabled',
		script_interpreters: [],
		...overrides
	};
}

describe('the Hosts screen', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		session.role = 'admin';
		mocks.listAgents.mockResolvedValue([]);
		mocks.listEnrollmentTokens.mockResolvedValue([]);
	});

	it('shows only a registration that can still enroll a host', async () => {
		mocks.listEnrollmentTokens.mockResolvedValue([
			token({ id: 'e_1', display_prefix: 'ab12cd34', labels: ['web'] }),
			token({ id: 'e_2', display_prefix: 'used0000', used_at: PAST }),
			token({ id: 'e_3', display_prefix: 'revoke00', revoked_at: PAST }),
			token({ id: 'e_4', display_prefix: 'expire00', expires_at: PAST })
		]);
		render(Hosts);

		await waitFor(() => expect(screen.getByText('Pending registrations')).toBeInTheDocument());
		expect(screen.getByText('ab12cd34')).toBeInTheDocument();
		expect(screen.getByText('web')).toBeInTheDocument();
		expect(screen.queryByText('used0000')).not.toBeInTheDocument();
		expect(screen.queryByText('revoke00')).not.toBeInTheDocument();
		expect(screen.queryByText('expire00')).not.toBeInTheDocument();
	});

	it('cancels a pending registration and the row disappears', async () => {
		const user = userEvent.setup();
		mocks.listEnrollmentTokens.mockResolvedValueOnce([token()]).mockResolvedValueOnce([]);
		render(Hosts);

		await screen.findByText('Pending registrations');
		await user.click(screen.getByRole('button', { name: 'Cancel' }));
		const dialog = await screen.findByRole('dialog');
		await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));

		await waitFor(() => expect(mocks.revokeEnrollmentToken).toHaveBeenCalledWith('e_1'));
		await waitFor(() =>
			expect(screen.queryByText('Pending registrations')).not.toBeInTheDocument()
		);
	});

	it('hides every write action from a Viewer', async () => {
		session.role = 'viewer';
		mocks.listAgents.mockResolvedValue([agent()]);
		mocks.listEnrollmentTokens.mockResolvedValue([token()]);
		render(Hosts);

		await screen.findByText('ab12cd34');
		expect(screen.queryByRole('button', { name: 'Add host' })).not.toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Cancel' })).not.toBeInTheDocument();
		expect(
			screen.queryByRole('button', { name: 'More actions for win-fs01' })
		).not.toBeInTheDocument();
	});

	it('renames a host and refuses an empty name', async () => {
		const user = userEvent.setup();
		mocks.listAgents.mockResolvedValue([agent()]);
		mocks.renameAgent.mockResolvedValue(agent({ hostname: 'new-name' }));
		render(Hosts);

		await screen.findByText('win-fs01');
		await user.click(screen.getByRole('button', { name: 'More actions for win-fs01' }));
		// The menu content of bits-ui is `visibility: hidden` in jsdom, so a role query cannot see
		// the item. The text query ignores the visibility, as the component test does.
		await user.click(await screen.findByText('Rename'));

		const dialog = await screen.findByRole('dialog');
		const input = document.querySelector<HTMLInputElement>('#host-rename') as HTMLInputElement;
		expect(input.value).toBe('win-fs01');

		await user.clear(input);
		await user.type(input, '   ');
		await waitFor(() =>
			expect(within(dialog).getByRole('button', { name: 'Rename' })).toBeDisabled()
		);

		await user.clear(input);
		await user.type(input, '  new-name  ');
		await user.click(within(dialog).getByRole('button', { name: 'Rename' }));

		await waitFor(() => expect(mocks.renameAgent).toHaveBeenCalledWith('a_01', 'new-name'));
	});

	it('shows the trust mode of each host and the command that turns it on (FR-HK-08)', async () => {
		const user = userEvent.setup();
		mocks.listAgents.mockResolvedValue([
			agent({ id: 'a_01', hostname: 'win-fs01', os: 'windows' }),
			agent({
				id: 'a_02',
				hostname: 'srv-db-01',
				os: 'linux',
				server_scripts: 'enabled',
				script_interpreters: ['bash', 'sh']
			})
		]);
		render(Hosts);

		await screen.findByText('win-fs01');
		expect(screen.getByText('Enabled')).toBeInTheDocument();
		await user.click(screen.getByRole('button', { name: 'Server scripts: win-fs01' }));
		let dialog = await screen.findByRole('dialog');
		expect(
			within(dialog).getByDisplayValue('restichub-agent.exe trust-server-scripts --enable')
		).toBeInTheDocument();
		await user.keyboard('{Escape}');
		await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());

		await user.click(screen.getByRole('button', { name: 'Server scripts: srv-db-01' }));
		dialog = await screen.findByRole('dialog', { name: 'Server scripts' });
		expect(within(dialog).getByText('bash, sh')).toBeInTheDocument();
		expect(
			within(dialog).getByText(
				'The control plane then runs code on this host with the rights of the agent.'
			)
		).toBeInTheDocument();
	});

	it('marks an agent that needs an update and an agent with an unsupported version (FR-PR-09)', async () => {
		mocks.listAgents.mockResolvedValue([
			agent({ id: 'a_01', hostname: 'host-current' }),
			agent({
				id: 'a_02',
				hostname: 'host-old',
				agent_version: '0.3.0',
				version_status: 'update_available'
			}),
			agent({
				id: 'a_03',
				hostname: 'host-refused',
				agent_version: '0.1.0',
				protocol_version: '0.9',
				version_status: 'unsupported'
			})
		]);
		render(Hosts);

		await screen.findByText('host-refused');
		expect(screen.getAllByText('Update available')).toHaveLength(1);
		expect(screen.getAllByText('Unsupported version')).toHaveLength(1);
		expect(screen.getByText('0.1.0')).toHaveAttribute(
			'title',
			'Protocol 0.9 · control plane 0.4.2'
		);
	});

	it('hides the update badge of a revoked host (FR-PR-09)', async () => {
		mocks.listAgents.mockResolvedValue([
			agent({ id: 'a_01', hostname: 'host-live', version_status: 'update_available' }),
			agent({
				id: 'a_02',
				hostname: 'host-gone',
				version_status: 'update_available',
				status: 'revoked',
				revoked_at: PAST
			})
		]);
		render(Hosts);

		await screen.findByText('host-gone');
		expect(screen.getAllByText('Update available')).toHaveLength(1);
	});

	it('warns about a host that presented a replaced certificate (T17)', async () => {
		mocks.listAgents.mockResolvedValue([
			agent({ id: 'a_01', hostname: 'host-clean' }),
			agent({ id: 'a_02', hostname: 'host-clone', superseded_certificate_used_at: PAST })
		]);
		render(Hosts);

		await screen.findByText('host-clone');
		expect(screen.getAllByText('Old certificate used')).toHaveLength(1);
	});

	it('shows the certificate expiry, the last rotation and warns before the expiry (FR-ID-03)', async () => {
		const soon = new Date(Date.now() + 2 * 24 * 60 * 60 * 1000).toISOString();
		const rotated = new Date(Date.now() - 3 * 24 * 60 * 60 * 1000).toISOString();
		mocks.listAgents.mockResolvedValue([
			agent({ id: 'a_01', hostname: 'host-fresh', last_certificate_rotation_at: rotated }),
			agent({ id: 'a_02', hostname: 'host-soon', certificate_expires_at: soon })
		]);
		render(Hosts);

		await screen.findByText('host-soon');
		expect(screen.getAllByText(/^Expires /)).toHaveLength(2);
		expect(screen.getAllByText('Not renewed yet')).toHaveLength(1);
		expect(screen.getAllByText(/^Renewed /)).toHaveLength(1);
		const [fresh, warn] = screen.getAllByText(/^Expires /);
		expect(fresh.classList.contains('cert__expiry--warn')).toBe(false);
		expect(warn.classList.contains('cert__expiry--warn')).toBe(true);
	});
});
