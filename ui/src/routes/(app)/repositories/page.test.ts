/*
 * The Repositories screen shows the maintenance of a repository: the warning for a missing
 * maintenance host, the schedules in the dialog and the run-now actions (FR-RP-08, FR-SC-09).
 *
 * The test replaces the session store, the toast store and the service, so it renders the screen
 * alone.
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Repositories from './+page.svelte';

const mocks = vi.hoisted(() => ({
	listRepositories: vi.fn(),
	listAgents: vi.fn(),
	updateRepository: vi.fn(),
	getMaintenance: vi.fn(),
	updateMaintenance: vi.fn(),
	runMaintenance: vi.fn(),
	unlockAllRepository: vi.fn(),
	previewSchedule: vi.fn(),
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
			listRepositories: mocks.listRepositories,
			listAgents: mocks.listAgents,
			updateRepository: mocks.updateRepository,
			getMaintenance: mocks.getMaintenance,
			updateMaintenance: mocks.updateMaintenance,
			runMaintenance: mocks.runMaintenance,
			unlockAllRepository: mocks.unlockAllRepository,
			previewSchedule: mocks.previewSchedule
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
		...overrides
	};
}

function maintenance(overrides: Record<string, unknown> = {}) {
	return {
		repository_id: 'r_1',
		maintenance_agent_id: 'a_1',
		retention: {
			cron: '0 3 * * 0',
			time_zone: 'UTC',
			enabled: true,
			next_run_at: '2999-01-04T03:00:00Z',
			last_fire_time: null
		},
		check: {
			cron: '0 4 1 * *',
			time_zone: 'UTC',
			enabled: true,
			next_run_at: '2999-02-01T04:00:00Z',
			last_fire_time: null
		},
		last_retention: null,
		last_check: null,
		...overrides
	};
}

describe('the Repositories screen maintenance', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		session.role = 'admin';
		mocks.listRepositories.mockResolvedValue([repository()]);
		mocks.listAgents.mockResolvedValue([{ id: 'a_1', hostname: 'nas-backup', status: 'active' }]);
		mocks.getMaintenance.mockResolvedValue(maintenance());
		mocks.previewSchedule.mockResolvedValue({
			time_zone: 'UTC',
			runs: ['2999-01-04T03:00:00Z']
		});
	});

	it('fr_rp_08 warns about a repository without a maintenance host', async () => {
		mocks.listRepositories.mockResolvedValue([repository({ maintenance_agent_id: null })]);
		render(Repositories);

		await screen.findByText('repo-a');
		expect(screen.getByText('No maintenance host')).toBeInTheDocument();
	});

	it('fr_rp_08 saves the schedules with the repository', async () => {
		const user = userEvent.setup();
		mocks.updateRepository.mockResolvedValue(repository());
		mocks.updateMaintenance.mockResolvedValue(maintenance());
		render(Repositories);

		await screen.findByText('repo-a');
		await user.click(screen.getByRole('button', { name: 'More actions for repo-a' }));
		await user.click(await screen.findByText('Edit'));
		const dialog = await screen.findByRole('dialog');
		const cron = await within(dialog).findByLabelText(/Cron expression/, {
			selector: '#maintenance-retention-cron'
		});
		await user.clear(cron);
		await user.type(cron, '0 2 * * 6');
		await user.click(within(dialog).getByRole('button', { name: 'Save' }));

		await waitFor(() =>
			expect(mocks.updateMaintenance).toHaveBeenCalledWith('r_1', {
				time_zone: 'UTC',
				retention: { cron: '0 2 * * 6', enabled: true },
				check: { cron: '0 4 1 * *', enabled: true }
			})
		);
	});

	it('fr_rp_08 shows the warning in the dialog when the host is cleared', async () => {
		const user = userEvent.setup();
		render(Repositories);

		await screen.findByText('repo-a');
		await user.click(screen.getByRole('button', { name: 'More actions for repo-a' }));
		await user.click(await screen.findByText('Edit'));
		const dialog = await screen.findByRole('dialog');
		await within(dialog).findByText('Retention');
		expect(
			within(dialog).queryByText(/creates no forget, prune or check job/)
		).not.toBeInTheDocument();
		await user.selectOptions(dialog.querySelector('#repo-agent') as HTMLSelectElement, '');

		expect(
			await within(dialog).findByText(/creates no forget, prune or check job/)
		).toBeInTheDocument();
	});

	it('fr_sc_09 lets an Operator run a maintenance task now', async () => {
		const user = userEvent.setup();
		session.role = 'operator';
		mocks.runMaintenance.mockResolvedValue({ id: 'j_1', status: 'queued' });
		render(Repositories);

		await screen.findByText('repo-a');
		expect(screen.queryByRole('button', { name: 'Add repository' })).not.toBeInTheDocument();
		await user.click(screen.getByRole('button', { name: 'More actions for repo-a' }));
		expect(screen.queryByText('Edit')).not.toBeInTheDocument();
		await user.click(await screen.findByText('Run retention now'));

		await waitFor(() => expect(mocks.runMaintenance).toHaveBeenCalledWith('r_1', 'retention'));
		await waitFor(() =>
			expect(mocks.pushToast).toHaveBeenCalledWith('success', expect.stringContaining('queued'))
		);
	});

	it('fr_rp_08 defaults the maintenance time zone of a new repository to the browser zone', async () => {
		const user = userEvent.setup();
		render(Repositories);

		await screen.findByText('repo-a');
		await user.click(screen.getByRole('button', { name: 'Add repository' }));
		const dialog = await screen.findByRole('dialog', { name: 'Add a repository' });

		const zone = Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
		expect(within(dialog).getByLabelText(/Time zone/)).toHaveValue(zone);
		expect(within(dialog).getAllByLabelText(/Cron expression/)[0]).toHaveValue('0 3 * * 0');
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
		const user = userEvent.setup();
		session.role = 'operator';
		render(Repositories);

		await screen.findByText('repo-a');
		await user.click(screen.getByRole('button', { name: 'More actions for repo-a' }));
		await screen.findByText('Run retention now');
		expect(screen.queryByText('Unlock all')).not.toBeInTheDocument();
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
