/*
 * The Plans screen lists the plans of the organization, links each plan to its page, runs one
 * now and deletes one (FR-SC-01, FR-SC-07, FR-SC-09).
 *
 * The test replaces the session store, the toast store and the service, so it renders the screen
 * alone. Only an Admin and an Operator see a write action; a Viewer reads.
 */
import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '#lib/api/index.ts';
import Plans from './+page.svelte';

const mocks = vi.hoisted(() => ({
	listPlans: vi.fn(),
	listProfiles: vi.fn(),
	listRepositories: vi.fn(),
	listAgents: vi.fn(),
	deletePlan: vi.fn(),
	runPlan: vi.fn(),
	updatePlan: vi.fn(),
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
			listPlans: mocks.listPlans,
			listProfiles: mocks.listProfiles,
			listRepositories: mocks.listRepositories,
			listAgents: mocks.listAgents,
			deletePlan: mocks.deletePlan,
			runPlan: mocks.runPlan,
			updatePlan: mocks.updatePlan
		}
	};
});

const FUTURE = '2999-01-01T02:00:00Z';

function plan(overrides: Record<string, unknown> = {}) {
	return {
		id: 'pl_1',
		name: 'nightly-web',
		profile_id: 'p_1',
		profile_name: 'web-daily',
		repository_id: 'r_1',
		agent_id: 'a_1',
		agent_hostname: 'win-fs01',
		cron: '0 2 * * *',
		time_zone: 'Europe/Berlin',
		missed_run_policy: 'latest_only',
		enabled: true,
		rpo_seconds: null,
		effective_rpo_seconds: 93600,
		next_run_at: FUTURE,
		last_fire_time: null,
		last_success_at: null,
		rpo_state: 'ok',
		created_at: FUTURE,
		updated_at: FUTURE,
		...overrides
	};
}

describe('the Plans screen', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		session.role = 'admin';
		mocks.listPlans.mockResolvedValue([]);
		mocks.listProfiles.mockResolvedValue([
			{ id: 'p_1', name: 'web-daily', paths: { all: ['/srv'] }, retention: null },
			{ id: 'p_2', name: 'nas-files', paths: { all: ['/srv/files'] }, retention: null }
		]);
		mocks.listRepositories.mockResolvedValue([
			{
				id: 'r_1',
				name: 'test-s3-yandex',
				backend: { kind: 's3', endpoint: 'storage.example.net', bucket: 'b' },
				initialized: true
			},
			{
				id: 'r_2',
				name: 'nas-local',
				backend: { kind: 'local', path: '/srv/restic' },
				maintenance_agent_id: 'a_2',
				initialized: true
			}
		]);
		mocks.listAgents.mockResolvedValue([{ id: 'a_2', hostname: 'MINDCORE' }]);
	});

	it('adr_0047 puts the repository third in line 2 and filters by it', async () => {
		const user = userEvent.setup();
		mocks.listPlans.mockResolvedValue([
			plan(),
			plan({
				id: 'pl_2',
				name: 'nas-nightly',
				profile_id: 'p_2',
				profile_name: 'nas-files',
				repository_id: 'r_2'
			})
		]);
		render(Plans);

		const item = (await screen.findByText('nightly-web')).closest('li') as HTMLElement;
		expect(item).toHaveTextContent('win-fs01 · web-daily · test-s3-yandex ·');

		await user.click(screen.getByRole('button', { name: /^Repository/ }));
		const nas = await screen.findByLabelText('nas-local');
		expect(document.getElementById(nas.getAttribute('aria-describedby') ?? '')).toHaveTextContent(
			/Local\s*·\s*\/srv\/restic\s*·\s*host MINDCORE/
		);
		await user.click(nas);
		await waitFor(() => expect(screen.queryByText('nightly-web')).not.toBeInTheDocument());
		expect(screen.getByText('nas-nightly')).toBeInTheDocument();
	});

	it('searches the repository name of a plan', async () => {
		const user = userEvent.setup();
		mocks.listPlans.mockResolvedValue([
			plan(),
			plan({
				id: 'pl_2',
				name: 'nas-nightly',
				profile_id: 'p_2',
				profile_name: 'nas-files',
				repository_id: 'r_2'
			})
		]);
		render(Plans);

		await screen.findByText('nas-nightly');
		await user.type(screen.getByPlaceholderText('Search plans'), 'nas-local');
		await waitFor(() => expect(screen.queryByText('nightly-web')).not.toBeInTheDocument());
		expect(screen.getByText('nas-nightly')).toBeInTheDocument();
	});

	it('lists a plan with its host, its profile, its repository and its schedule', async () => {
		mocks.listPlans.mockResolvedValue([plan()]);
		render(Plans);

		await waitFor(() => expect(screen.getByText('nightly-web')).toBeInTheDocument());
		expect(screen.getByText('win-fs01')).toBeInTheDocument();
		expect(screen.getByText('web-daily')).toBeInTheDocument();
		expect(screen.getByText('test-s3-yandex')).toBeInTheDocument();
		// The schedule shows the phrase; the cron expression and the zone are in its tooltip.
		expect(screen.getByText('every day at 02:00')).toHaveAttribute(
			'title',
			'0 2 * * * · Europe/Berlin'
		);
	});

	it('opens the new plan page from the add button', async () => {
		const user = userEvent.setup();
		render(Plans);

		await screen.findByText('No plans yet');
		await user.click(screen.getByRole('button', { name: 'Add plan' }));
		expect(mocks.goto).toHaveBeenCalledWith('/plans/new');
	});

	it('runs a plan now and shows no queue note', async () => {
		const user = userEvent.setup();
		mocks.listPlans.mockResolvedValue([plan()]);
		mocks.runPlan.mockResolvedValue({ id: 'j_1', status: 'queued' });
		render(Plans);

		await screen.findByText('nightly-web');
		expect(screen.queryByText(/dispatcher/i)).not.toBeInTheDocument();
		await user.click(screen.getByRole('button', { name: 'Run now' }));

		await waitFor(() => expect(mocks.runPlan).toHaveBeenCalledWith('pl_1'));
		await waitFor(() =>
			expect(mocks.pushToast).toHaveBeenCalledWith('success', expect.stringContaining('queued'))
		);
	});

	it('ac_rp_07 shows why a plan on another host than its local repository does not run', async () => {
		const user = userEvent.setup();
		mocks.listPlans.mockResolvedValue([plan()]);
		mocks.runPlan.mockRejectedValue(
			new ApiError({ status: 409, code: 'not_repository_host', title: 'Conflict' })
		);
		render(Plans);

		await screen.findByText('nightly-web');
		await user.click(screen.getByRole('button', { name: 'Run now' }));

		await waitFor(() =>
			expect(mocks.pushToast).toHaveBeenCalledWith(
				'danger',
				expect.stringContaining('another host than its local repository')
			)
		);
	});

	it('hides every write action from a Viewer', async () => {
		session.role = 'viewer';
		mocks.listPlans.mockResolvedValue([plan()]);
		render(Plans);

		await screen.findByText('nightly-web');
		expect(screen.queryByRole('button', { name: 'Add plan' })).not.toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Run now' })).not.toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Delete plan' })).not.toBeInTheDocument();
	});

	it('links the plan name to the plan page', async () => {
		mocks.listPlans.mockResolvedValue([plan()]);
		render(Plans);

		const link = await screen.findByRole('link', { name: 'nightly-web' });
		expect(link).toHaveAttribute('href', '/plans/pl_1');
	});

	it('disables a plan from its item and keeps delete on the plan page', async () => {
		const user = userEvent.setup();
		mocks.listPlans.mockResolvedValue([plan()]);
		mocks.updatePlan.mockResolvedValue({});
		render(Plans);

		await screen.findByText('nightly-web');
		expect(screen.queryByRole('button', { name: 'Delete plan' })).not.toBeInTheDocument();
		await user.click(screen.getByRole('button', { name: 'Disable' }));
		await waitFor(() =>
			expect(mocks.updatePlan).toHaveBeenCalledWith(
				'pl_1',
				expect.objectContaining({
					enabled: false,
					cron: '0 2 * * *',
					profile_id: 'p_1',
					repository_id: 'r_1'
				})
			)
		);
	});

	it('fr_ui_01 shows the recovery point state and the last backup of each plan', async () => {
		const hoursAgo = (hours: number) => new Date(Date.now() - hours * 3_600_000).toISOString();
		mocks.listPlans.mockResolvedValue([
			plan({ last_success_at: hoursAgo(7) }),
			plan({ id: 'pl_2', name: 'late', rpo_state: 'at_risk', last_success_at: hoursAgo(22) }),
			plan({ id: 'pl_3', name: 'stale', rpo_state: 'violated' }),
			plan({ id: 'pl_4', name: 'off', rpo_state: 'violated', enabled: false })
		]);
		const { container } = render(Plans);

		await screen.findByText('late');
		expect(screen.getByText('backup 7 h ago')).toBeInTheDocument();
		expect(screen.getByText('backup 22 h ago')).toBeInTheDocument();
		expect(screen.getAllByText('no backup yet')).toHaveLength(2);
		expect(screen.getByText('At risk', { selector: '.status' })).toHaveClass('status--warning');
		expect(screen.getByText('Backup overdue', { selector: '.status' })).toHaveClass(
			'status--failed'
		);
		// A disabled plan shows Disabled, not its overdue backup.
		expect(container.querySelectorAll('.status')).toHaveLength(3);
		expect(screen.getByRole('button', { name: 'Need attention 2' })).toBeInTheDocument();
	});

	it('shows the Disabled pill and filters the disabled plans with a tab', async () => {
		const user = userEvent.setup();
		mocks.listPlans.mockResolvedValue([plan(), plan({ id: 'pl_2', name: 'old', enabled: false })]);
		render(Plans);

		await screen.findByText('old');
		expect(screen.getAllByText('Disabled').some((node) => node.closest('.status'))).toBe(true);
		await user.click(screen.getByRole('button', { name: 'Disabled 1' }));
		expect(mocks.goto).toHaveBeenCalled();
	});
});
