/*
 * The repository page: the sections, the create-only password, the schedules with the compact
 * next runs and the unsaved-changes state (FR-RP-01, FR-RP-08, FR-SC-01).
 */
import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import RepositoryEditor from './RepositoryEditor.svelte';

const mocks = vi.hoisted(() => ({
	listRepositories: vi.fn(),
	listAgents: vi.fn(),
	createRepository: vi.fn(),
	updateRepository: vi.fn(),
	getMaintenance: vi.fn(),
	updateMaintenance: vi.fn(),
	previewSchedule: vi.fn(),
	pushToast: vi.fn(),
	goto: vi.fn()
}));

const session = vi.hoisted(() => ({ role: 'admin' as string }));

vi.mock('$app/paths', () => ({
	resolve: (path: string, params?: Record<string, string>) =>
		params ? path.replace('[id]', params.id) : path
}));
vi.mock('$app/navigation', () => ({ goto: mocks.goto, beforeNavigate: vi.fn() }));
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
			listRepositories: mocks.listRepositories,
			listAgents: mocks.listAgents,
			createRepository: mocks.createRepository,
			updateRepository: mocks.updateRepository,
			getMaintenance: mocks.getMaintenance,
			updateMaintenance: mocks.updateMaintenance,
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

function maintenance() {
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
		last_check: null
	};
}

describe('the repository page', () => {
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

	it('fr_rp_08 saves the schedules with the repository', async () => {
		const user = userEvent.setup();
		mocks.updateRepository.mockResolvedValue(repository());
		mocks.updateMaintenance.mockResolvedValue(maintenance());
		render(RepositoryEditor, { repositoryId: 'r_1' });

		const cron = await screen.findByLabelText(/Cron expression/, {
			selector: '#maintenance-retention-cron'
		});
		await user.clear(cron);
		await user.type(cron, '0 2 * * 6');
		await user.click(screen.getByRole('button', { name: 'Save' }));

		await waitFor(() =>
			expect(mocks.updateMaintenance).toHaveBeenCalledWith('r_1', {
				time_zone: 'UTC',
				retention: { cron: '0 2 * * 6', enabled: true },
				check: { cron: '0 4 1 * *', enabled: true }
			})
		);
		await waitFor(() => expect(mocks.goto).toHaveBeenCalledWith('/(app)/repositories'));
	});

	it('fr_rp_08 shows the warning when the maintenance host is cleared', async () => {
		const user = userEvent.setup();
		const { container } = render(RepositoryEditor, { repositoryId: 'r_1' });

		await screen.findByLabelText('Maintenance host');
		expect(screen.queryByText(/creates no forget, prune or check job/)).not.toBeInTheDocument();
		await user.selectOptions(container.querySelector('#repo-agent') as HTMLSelectElement, '');

		expect(await screen.findByText(/creates no forget, prune or check job/)).toBeInTheDocument();
	});

	it('fr_rp_08 defaults the time zone of a new repository to the browser zone', async () => {
		render(RepositoryEditor);

		const zone = Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
		expect(await screen.findByLabelText(/Time zone/)).toHaveValue(zone);
		expect(screen.getAllByLabelText(/Cron expression/)[0]).toHaveValue('0 3 * * 0');
	});

	it('fr_rp_05 offers the password at creation only', async () => {
		const { unmount } = render(RepositoryEditor);
		expect(await screen.findByLabelText(/Repository password/)).toBeInTheDocument();
		unmount();

		render(RepositoryEditor, { repositoryId: 'r_1' });
		await screen.findByLabelText('Maintenance host');
		expect(screen.queryByLabelText(/Repository password/)).not.toBeInTheDocument();
		expect(screen.getByText(/cannot be replaced/)).toBeInTheDocument();
	});

	it('fr_sc_01 lists the next runs on one line each with the UTC time in the tooltip', async () => {
		render(RepositoryEditor, { repositoryId: 'r_1' });

		const run = await screen.findAllByText(/2999/);
		const item = run.find((node) => node.tagName === 'LI');
		expect(item).toBeDefined();
		expect(item?.getAttribute('title')).toContain('UTC');
	});

	it('fr_rp_01 marks unsaved changes and sends a new repository', async () => {
		const user = userEvent.setup();
		mocks.createRepository.mockResolvedValue(repository());
		mocks.updateMaintenance.mockResolvedValue(maintenance());
		render(RepositoryEditor);

		await user.type(await screen.findByLabelText(/^Repository\b/, { selector: '#repo-name' }), 'x');
		expect(screen.getByText('The changes are not saved.')).toBeInTheDocument();
		await user.selectOptions(screen.getByLabelText('Backend kind'), 'local');
		await user.type(screen.getByLabelText(/Path/), '/srv/x');
		await user.type(screen.getByLabelText(/Repository password/), 'pw');
		await user.click(screen.getByRole('button', { name: 'Save' }));

		await waitFor(() =>
			expect(mocks.createRepository).toHaveBeenCalledWith(
				expect.objectContaining({
					name: 'x',
					password: 'pw',
					backend: { kind: 'local', path: '/srv/x' }
				})
			)
		);
	});

	it('fr_rp_01 says "Not set" for the empty prefix of an initialized S3 repository', async () => {
		mocks.listRepositories.mockResolvedValue([
			repository({
				backend: { kind: 's3', endpoint: 'https://s3.example.net', bucket: 'backups' }
			})
		]);
		render(RepositoryEditor, { repositoryId: 'r_1' });

		const prefix = await screen.findByLabelText('Prefix');
		expect(prefix).toBeDisabled();
		expect(prefix).toHaveValue('');
		expect(prefix).not.toHaveAttribute('placeholder');
		expect(prefix).toHaveAccessibleDescription('Not set');
		expect(screen.getByLabelText(/Bucket/)).toHaveValue('backups');
	});

	it('fr_rp_01 reports a repository that does not exist', async () => {
		render(RepositoryEditor, { repositoryId: 'r_9' });

		expect(await screen.findByText('The repository does not exist.')).toBeInTheDocument();
	});
});
