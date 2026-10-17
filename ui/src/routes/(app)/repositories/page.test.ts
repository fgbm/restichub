/*
 * The Repositories screen shows the maintenance of a repository: the warning for a missing
 * maintenance host, the run-now actions and the links to the repository page (FR-RP-08, FR-SC-09).
 *
 * The test replaces the session store, the toast store and the service, so it renders the screen
 * alone.
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Repositories from './+page.svelte';

const HOST = {
	status: 'active',
	os: 'linux',
	arch: 'amd64',
	labels: [],
	online: true,
	first_connected_at: '2026-01-01T00:00:00Z',
	last_seen_at: '2026-01-01T00:00:00Z'
};

const mocks = vi.hoisted(() => ({
	listRepositories: vi.fn(),
	listAgents: vi.fn(),
	runMaintenance: vi.fn(),
	unlockAllRepository: vi.fn(),
	initRepository: vi.fn(),
	testAccessRepository: vi.fn(),
	getJob: vi.fn(),
	pushToast: vi.fn(),
	goto: vi.fn()
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

vi.mock('$app/navigation', () => ({ goto: mocks.goto }));
vi.mock('#lib/stores/toasts.svelte.ts', () => ({ pushToast: mocks.pushToast }));

vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		service: {
			...actual.service,
			listRepositories: mocks.listRepositories,
			listAgents: mocks.listAgents,
			runMaintenance: mocks.runMaintenance,
			unlockAllRepository: mocks.unlockAllRepository,
			initRepository: mocks.initRepository,
			testAccessRepository: mocks.testAccessRepository,
			getJob: mocks.getJob
		}
	};
});

function repository(overrides: Record<string, unknown> = {}) {
	return {
		id: 'r_1',
		name: 'repo-a',
		backend: { kind: 'local', path: '/srv/backup/restic' },
		credentials: {
			password: true,
			s3_access_key: false,
			s3_secret_key: false,
			rest_password: false
		},
		initialized: true,
		maintenance_agent_id: 'a_1',
		created_at: '2026-10-01T00:00:00Z',
		updated_at: '2026-10-01T00:00:00Z',
		created_by: 'u_01',
		lock_state: 'free',
		last_check: null,
		snapshot_count: null,
		shares_location_with: [],
		...overrides
	};
}

describe('the Repositories screen maintenance', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		session.role = 'admin';
		mocks.listRepositories.mockResolvedValue([repository()]);
		mocks.listAgents.mockResolvedValue([{ id: 'a_1', hostname: 'nas-backup', ...HOST }]);
	});

	it('fr_rp_08 warns about a repository without a maintenance host', async () => {
		mocks.listRepositories.mockResolvedValue([repository({ maintenance_agent_id: null })]);
		render(Repositories);

		await screen.findByText('repo-a');
		expect(screen.getByText('no maintenance host')).toBeInTheDocument();
	});

	it('fr_ui_01 shows the last check, the snapshot count and the state of the repository', async () => {
		const daysAgo = (days: number) => new Date(Date.now() - days * 86_400_000).toISOString();
		mocks.listRepositories.mockResolvedValue([
			repository({
				last_check: { finished_at: daysAgo(2), result: 'passed' },
				snapshot_count: 146
			}),
			repository({
				id: 'r_2',
				name: 'repo-b',
				lock_state: 'stale',
				last_check: { finished_at: daysAgo(6), result: 'passed' }
			}),
			repository({
				id: 'r_3',
				name: 'repo-c',
				lock_state: 'stale',
				last_check: { finished_at: daysAgo(1), result: 'failed' }
			})
		]);
		render(Repositories);

		await screen.findByText('repo-a');
		expect(screen.getByText('checked 2 d ago')).toBeInTheDocument();
		expect(screen.getByText('146 snapshots')).toBeInTheDocument();
		expect(screen.getByText('Stale lock', { selector: '.status' })).toHaveClass('status--warning');
		// A failed check outranks the stale lock of the same repository.
		expect(screen.getByText('Check failed', { selector: '.status' })).toHaveClass('status--failed');
		expect(screen.getByRole('button', { name: 'Need attention 2' })).toBeInTheDocument();
	});

	it('fr_ui_01 says that a repository was never checked', async () => {
		render(Repositories);
		await screen.findByText('repo-a');
		expect(screen.getByText('not checked')).toBeInTheDocument();
		expect(screen.queryByText(/snapshots?$/)).not.toBeInTheDocument();
	});

	it('fr_rp_08 opens the repository page from the row and has no Edit action', async () => {
		const user = userEvent.setup();
		render(Repositories);

		const link = await screen.findByRole('link', { name: 'repo-a' });
		expect(link).toHaveAttribute('href', '/repositories/r_1');
		await user.click(screen.getByRole('button', { name: 'Add repository' }));
		expect(mocks.goto).toHaveBeenCalledWith('/repositories/new');
		// Opening the record is not a row action (design/style/ui-principles.md, 2.3).
		await user.click(screen.getByRole('button', { name: 'More actions for repo-a' }));
		await screen.findByText('Test access');
		expect(screen.queryByText('Edit')).not.toBeInTheDocument();
	});

	it('fr_sc_09 lets an Operator run a maintenance task now', async () => {
		const user = userEvent.setup();
		session.role = 'operator';
		mocks.runMaintenance.mockResolvedValue({ id: 'j_1', status: 'queued' });
		render(Repositories);

		await screen.findByText('repo-a');
		expect(screen.queryByRole('button', { name: 'Add repository' })).not.toBeInTheDocument();
		expect(
			screen.queryByRole('button', { name: 'More actions for repo-a' })
		).not.toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Edit' })).not.toBeInTheDocument();
		await user.click(screen.getByRole('button', { name: 'Delete expired snapshots now' }));

		await waitFor(() => expect(mocks.runMaintenance).toHaveBeenCalledWith('r_1', 'retention'));
		await waitFor(() =>
			expect(mocks.pushToast).toHaveBeenCalledWith('success', expect.stringContaining('queued'))
		);
	});

	it('fr_sc_06 asks an Admin to type the repository name before it removes all locks', async () => {
		const user = userEvent.setup();
		mocks.unlockAllRepository.mockResolvedValue({ id: 'j_1', status: 'queued' });
		render(Repositories);

		await screen.findByText('repo-a');
		await user.click(screen.getByRole('button', { name: 'More actions for repo-a' }));
		await user.click(await screen.findByText('Unlock all'));

		const dialog = await screen.findByRole('dialog', { name: 'Remove all locks?' });
		expect(within(dialog).getByText(/locks of running operations/)).toBeInTheDocument();
		const confirm = within(dialog).getByRole('button', { name: 'Unlock all' });
		expect(confirm).toBeDisabled();
		const field = within(dialog).getByLabelText('Type the repository name to confirm');
		await user.type(field, 'repo-b');
		expect(confirm).toBeDisabled();
		await user.clear(field);
		await user.type(field, 'repo-a');
		expect(confirm).toBeEnabled();
		await user.click(confirm);

		await waitFor(() => expect(mocks.unlockAllRepository).toHaveBeenCalledWith('r_1', 'repo-a'));
		await waitFor(() =>
			expect(mocks.pushToast).toHaveBeenCalledWith('success', expect.stringContaining('queued'))
		);
	});

	it('fr_sc_06 offers no unlock-all action to an Operator', async () => {
		session.role = 'operator';
		render(Repositories);

		await screen.findByText('repo-a');
		await screen.findByRole('button', { name: 'Delete expired snapshots now' });
		expect(screen.queryByRole('button', { name: 'Unlock all' })).not.toBeInTheDocument();
	});

	it('fr_sc_09 hides every action from a Viewer', async () => {
		session.role = 'viewer';
		render(Repositories);

		await screen.findByText('repo-a');
		expect(
			screen.queryByRole('button', { name: 'More actions for repo-a' })
		).not.toBeInTheDocument();
	});
});

describe('the host of an init and an access test', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		session.role = 'admin';
		mocks.listAgents.mockResolvedValue([
			{ id: 'a_1', hostname: 'nas-backup', ...HOST },
			{ id: 'a_2', hostname: 'web-01', ...HOST }
		]);
	});

	async function openAction(label: string): Promise<HTMLElement> {
		const user = userEvent.setup();
		render(Repositories);
		await screen.findByText('repo-a');
		await user.click(screen.getByRole('button', { name: 'More actions for repo-a' }));
		await user.click(await screen.findByText(label));
		return screen.findByRole('dialog');
	}

	it('fr_rp_08 initializes on the maintenance host and does not offer another host', async () => {
		mocks.listRepositories.mockResolvedValue([repository({ initialized: false })]);
		mocks.initRepository.mockResolvedValue({ id: 'j_1', status: 'queued' });
		const dialog = await openAction('Initialize');

		const host = within(dialog).getByRole('combobox', { name: /^Host/ });
		expect(host).toHaveTextContent('nas-backup');
		expect(host).toBeDisabled();
		await userEvent.click(within(dialog).getByRole('button', { name: 'Start' }));

		await waitFor(() =>
			expect(mocks.initRepository).toHaveBeenCalledWith('r_1', { agent_id: 'a_1' })
		);
	});

	it('adr_0045 names the repository roots file when the host refuses a local path', async () => {
		const refused = {
			id: 'j_1',
			status: 'failed',
			result_code: 'failed',
			result_message: 'repository_path_denied'
		};
		mocks.listRepositories.mockResolvedValue([repository({ initialized: false })]);
		mocks.initRepository.mockResolvedValue({ id: 'j_1', status: 'queued' });
		mocks.getJob.mockResolvedValue(refused);
		const dialog = await openAction('Initialize');
		await userEvent.click(within(dialog).getByRole('button', { name: 'Start' }));

		expect(
			await within(dialog).findByText(/Add its root to \/etc\/restichub-agent-repository-roots.txt/)
		).toBeInTheDocument();
		expect(within(dialog).queryByText('The job failed.')).not.toBeInTheDocument();
	});

	it('fr_rp_08 says why an init cannot start without a maintenance host', async () => {
		mocks.listRepositories.mockResolvedValue([
			repository({ initialized: false, maintenance_agent_id: null })
		]);
		const dialog = await openAction('Initialize');

		expect(within(dialog).getByText('The repository has no maintenance host')).toBeInTheDocument();
		expect(
			within(dialog).getByRole('link', { name: 'Select the maintenance host' })
		).toHaveAttribute('href', '/repositories/r_1#repo-agent');
		expect(within(dialog).getByRole('button', { name: 'Start' })).toBeDisabled();
		expect(mocks.initRepository).not.toHaveBeenCalled();
	});

	it('fr_rp_08 lets an access test of an S3 repository choose another host', async () => {
		mocks.listRepositories.mockResolvedValue([
			repository({ backend: { kind: 's3', endpoint: 'https://s3.example.net', bucket: 'b' } })
		]);
		const dialog = await openAction('Test access');

		const host = within(dialog).getByRole('combobox', { name: /^Host/ });
		expect(host).toHaveTextContent('nas-backup');
		expect(host).toBeEnabled();
	});
});
