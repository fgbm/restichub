/*
 * The repository page: the sections, the create-only password, the schedules with the compact
 * next runs and the unsaved-changes state (FR-RP-01, FR-RP-08, FR-SC-01).
 */
import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import RepositoryEditor from './RepositoryEditor.svelte';

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
		shares_location_with: [],
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

const hostField = () =>
	waitFor(() => {
		const field = document.querySelector<HTMLButtonElement>('#repo-agent');
		expect(field).toBeTruthy();
		return field as HTMLButtonElement;
	});
const hostOptionNames = () =>
	[...document.querySelectorAll('[role="option"] .rh-record__option-label')].map((label) =>
		label.textContent?.trim()
	);

async function pickHost(user: ReturnType<typeof userEvent.setup>, name: string): Promise<void> {
	await user.click(await hostField());
	const label = await screen.findByText(name, { selector: '.rh-record__option-label' });
	await user.click(label.closest('[role="option"]')!);
}

describe('the repository page', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		session.role = 'admin';
		mocks.listRepositories.mockResolvedValue([repository()]);
		mocks.listAgents.mockResolvedValue([{ id: 'a_1', hostname: 'nas-backup', ...HOST }]);
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
		mocks.listRepositories.mockResolvedValue([
			repository({ backend: { kind: 'rest', endpoint: 'https://rest.example.net/a' } })
		]);
		render(RepositoryEditor, { repositoryId: 'r_1' });

		expect(await hostField()).toHaveTextContent('nas-backup');
		expect(screen.queryByText(/creates no forget, prune or check job/)).not.toBeInTheDocument();
		await pickHost(user, 'None');

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
		await screen.findByLabelText(/Repository host/);
		expect(screen.queryByLabelText(/Repository password/)).not.toBeInTheDocument();
		expect(screen.getByText(/cannot be replaced/)).toBeInTheDocument();
	});

	it('shows the sentence of the repositories list and the hint of a stored secret', async () => {
		mocks.listRepositories.mockResolvedValue([
			repository({
				backend: {
					kind: 's3',
					endpoint: 'https://s3.example.net',
					bucket: 'backups-prod',
					region: 'eu-central-1'
				}
			})
		]);
		render(RepositoryEditor, { repositoryId: 'r_1' });
		await screen.findByRole('heading', { level: 1, name: 'repo-a' });
		expect(
			screen.getByText(
				'Each repository holds one restic backend. The control plane keeps the secrets and the maintenance host.'
			)
		).toBeInTheDocument();
		expect(screen.getAllByText('Leave the field empty to keep the current value.')).toHaveLength(2);
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
		expect(screen.getByText('The changes are not saved')).toBeInTheDocument();
		await user.selectOptions(screen.getByLabelText('Backend kind'), 'local');
		await user.type(screen.getByLabelText(/Path/), '/srv/x');
		await user.type(screen.getByLabelText(/Repository password/), 'pw');
		await pickHost(user, 'nas-backup');
		await user.click(screen.getByRole('button', { name: 'Save' }));

		await waitFor(() =>
			expect(mocks.createRepository).toHaveBeenCalledWith(
				expect.objectContaining({
					name: 'x',
					password: 'pw',
					backend: { kind: 'local', path: '/srv/x' },
					maintenance_agent_id: 'a_1'
				})
			)
		);
	});

	it('fr_rp_08 locks the host of an initialized local repository and says why', async () => {
		render(RepositoryEditor, { repositoryId: 'r_1' });

		const host = await hostField();
		expect(host).toBeDisabled();
		expect(host).toHaveTextContent(/nas-backup\s*· Linux · Online/);
		expect(host).toHaveAccessibleDescription(/initialized local repository cannot change/);
	});

	it('fr_rp_08 requires the host of a local repository and lets it change before the init', async () => {
		mocks.listRepositories.mockResolvedValue([repository({ initialized: false })]);
		mocks.listAgents.mockResolvedValue([
			{ id: 'a_1', hostname: 'nas-backup', ...HOST },
			{ id: 'a_2', hostname: 'nas-spare', ...HOST }
		]);
		render(RepositoryEditor, { repositoryId: 'r_1' });

		const user = userEvent.setup();
		const host = await hostField();
		expect(host).toBeEnabled();
		expect(host).toHaveAttribute('aria-required', 'true');
		expect(host).toHaveAccessibleDescription(/Every job of the repository runs on this host/);
		await user.click(host);
		await screen.findByText('nas-spare', { selector: '.rh-record__option-label' });
		expect(hostOptionNames()).toEqual(['nas-backup', 'nas-spare']);
	});

	it('fr_rp_08 shows the field error of a refused host change', async () => {
		const user = userEvent.setup();
		mocks.listRepositories.mockResolvedValue([repository({ initialized: false })]);
		mocks.listAgents.mockResolvedValue([
			{ id: 'a_1', hostname: 'nas-backup', ...HOST },
			{ id: 'a_2', hostname: 'nas-spare', ...HOST }
		]);
		const { ApiError } = await import('#lib/api/index.ts');
		mocks.updateRepository.mockRejectedValue(
			new ApiError({
				title: 'Bad Request',
				status: 400,
				code: 'invalid_argument',
				field: 'maintenance_agent_id',
				reason: 'plan_on_other_host'
			})
		);
		render(RepositoryEditor, { repositoryId: 'r_1' });

		await pickHost(user, 'nas-spare');
		await user.click(screen.getByRole('button', { name: 'Save' }));

		expect(
			await screen.findByText(/A plan of the repository runs on another host/)
		).toBeInTheDocument();
	});

	it('fr_rp_11 names the repository that holds the location the form tried to save', async () => {
		const user = userEvent.setup();
		mocks.listRepositories.mockResolvedValue([repository({ initialized: false })]);
		const { ApiError } = await import('#lib/api/index.ts');
		mocks.updateRepository.mockRejectedValue(
			new ApiError({
				title: 'Conflict',
				status: 409,
				code: 'repository_location_taken',
				field: 'backend.path',
				existingName: 'repo-b'
			})
		);
		render(RepositoryEditor, { repositoryId: 'r_1' });

		const path = await screen.findByLabelText(/Path/);
		await user.clear(path);
		await user.type(path, '/srv/backup/other');
		await user.click(screen.getByRole('button', { name: 'Save' }));

		expect(
			await screen.findByText(/The repository "repo-b" already uses this location/)
		).toBeInTheDocument();
	});

	it('fr_rp_11 warns about a repository that shares its location', async () => {
		mocks.listRepositories.mockResolvedValue([repository({ shares_location_with: ['repo-b'] })]);
		render(RepositoryEditor, { repositoryId: 'r_1' });

		expect(
			await screen.findByText(/Another repository uses this location: repo-b/)
		).toBeInTheDocument();
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
