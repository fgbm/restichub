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

const mocks = vi.hoisted(() => ({
	listRepositories: vi.fn(),
	listAgents: vi.fn(),
	runMaintenance: vi.fn(),
	unlockAllRepository: vi.fn(),
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
			unlockAllRepository: mocks.unlockAllRepository
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

describe('the Repositories screen maintenance', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		session.role = 'admin';
		mocks.listRepositories.mockResolvedValue([repository()]);
		mocks.listAgents.mockResolvedValue([{ id: 'a_1', hostname: 'nas-backup', status: 'active' }]);
	});

	it('fr_rp_08 warns about a repository without a maintenance host', async () => {
		mocks.listRepositories.mockResolvedValue([repository({ maintenance_agent_id: null })]);
		render(Repositories);

		await screen.findByText('repo-a');
		expect(screen.getByText('No maintenance host')).toBeInTheDocument();
	});

	it('fr_rp_08 opens the repository page from the name and from the menu', async () => {
		const user = userEvent.setup();
		render(Repositories);

		const link = await screen.findByRole('link', { name: 'repo-a' });
		expect(link).toHaveAttribute('href', '/repositories/r_1');
		await user.click(screen.getByRole('button', { name: 'Add repository' }));
		expect(mocks.goto).toHaveBeenCalledWith('/repositories/new');
		await user.click(screen.getByRole('button', { name: 'More actions for repo-a' }));
		await user.click(await screen.findByText('Edit'));
		expect(mocks.goto).toHaveBeenCalledWith('/repositories/r_1');
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
		await user.click(screen.getByRole('button', { name: 'Run retention now' }));

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
		await screen.findByRole('button', { name: 'Run retention now' });
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
