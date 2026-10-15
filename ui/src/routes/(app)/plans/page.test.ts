/*
 * The Plans screen lists the plans of the organization, creates one and deletes one
 * (FR-SC-01, FR-SC-07).
 *
 * The test replaces the session store, the toast store and the service, so it renders the screen
 * alone. Only an Admin and an Operator see a write action; a Viewer reads.
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Plans from './+page.svelte';

const mocks = vi.hoisted(() => ({
	listPlans: vi.fn(),
	listProfiles: vi.fn(),
	listAgents: vi.fn(),
	createPlan: vi.fn(),
	deletePlan: vi.fn(),
	runPlan: vi.fn(),
	nextRuns: vi.fn(),
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
			listPlans: mocks.listPlans,
			listProfiles: mocks.listProfiles,
			listAgents: mocks.listAgents,
			createPlan: mocks.createPlan,
			deletePlan: mocks.deletePlan,
			runPlan: mocks.runPlan,
			nextRuns: mocks.nextRuns,
			previewSchedule: mocks.previewSchedule
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
		created_at: FUTURE,
		updated_at: FUTURE,
		...overrides
	};
}

function profile() {
	return { id: 'p_1', name: 'web-daily' };
}

function agent() {
	return { id: 'a_1', hostname: 'win-fs01', status: 'active' };
}

describe('the Plans screen', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		session.role = 'admin';
		mocks.listPlans.mockResolvedValue([]);
		mocks.listProfiles.mockResolvedValue([profile()]);
		mocks.listAgents.mockResolvedValue([agent()]);
		mocks.previewSchedule.mockResolvedValue({ time_zone: 'Europe/Berlin', runs: [] });
	});

	it('lists a plan with its host and its schedule', async () => {
		mocks.listPlans.mockResolvedValue([plan()]);
		render(Plans);

		await waitFor(() => expect(screen.getByText('nightly-web')).toBeInTheDocument());
		expect(screen.getByText('win-fs01')).toBeInTheDocument();
		expect(screen.getByText('web-daily')).toBeInTheDocument();
		// The schedule column shows the phrase and the cron expression.
		expect(screen.getByText('Every day at 02:00')).toBeInTheDocument();
		expect(screen.getByText('0 2 * * *')).toBeInTheDocument();
	});

	it('creates a plan from the dialog', async () => {
		const user = userEvent.setup();
		mocks.listPlans.mockResolvedValueOnce([]).mockResolvedValueOnce([plan()]);
		mocks.createPlan.mockResolvedValue({ plan: plan() });
		render(Plans);

		await screen.findByText('No plans yet');
		await user.click(screen.getByRole('button', { name: 'Add plan' }));
		const dialog = await screen.findByRole('dialog');
		const input = (id: string) => document.querySelector<HTMLInputElement>(id) as HTMLInputElement;
		await user.type(input('#plan-name'), 'nightly-web');
		await user.type(input('#plan-cron'), '0 2 * * *');
		// The create dialog fills the time zone with the browser zone, so the test replaces it.
		await user.clear(input('#plan-time-zone'));
		await user.type(input('#plan-time-zone'), 'Europe/Berlin');
		await user.click(within(dialog).getByRole('button', { name: 'Save' }));

		await waitFor(() =>
			expect(mocks.createPlan).toHaveBeenCalledWith({
				name: 'nightly-web',
				profile_id: 'p_1',
				agent_id: 'a_1',
				cron: '0 2 * * *',
				time_zone: 'Europe/Berlin',
				missed_run_policy: 'run_all',
				enabled: true,
				rpo_seconds: null
			})
		);
		await waitFor(() => expect(screen.getByText('nightly-web')).toBeInTheDocument());
	});

	it('sends the objective of the plan in seconds (ADR-0035)', async () => {
		const user = userEvent.setup();
		mocks.listPlans.mockResolvedValueOnce([]).mockResolvedValueOnce([plan()]);
		mocks.createPlan.mockResolvedValue({ plan: plan() });
		render(Plans);

		await screen.findByText('No plans yet');
		await user.click(screen.getByRole('button', { name: 'Add plan' }));
		const dialog = await screen.findByRole('dialog');
		const input = (id: string) => document.querySelector<HTMLInputElement>(id) as HTMLInputElement;
		await user.type(input('#plan-name'), 'nightly-web');
		await user.type(input('#plan-cron'), '0 2 * * *');
		await user.clear(input('#plan-time-zone'));
		await user.type(input('#plan-time-zone'), 'Europe/Berlin');
		await user.type(input('#plan-rpo'), '24,5');
		await user.click(within(dialog).getByRole('button', { name: 'Save' }));

		await waitFor(() =>
			expect(mocks.createPlan).toHaveBeenCalledWith(expect.objectContaining({ rpo_seconds: 88200 }))
		);
	});

	it('runs a plan now and shows no queue note', async () => {
		const user = userEvent.setup();
		mocks.listPlans.mockResolvedValue([plan()]);
		mocks.runPlan.mockResolvedValue({ id: 'j_1', status: 'queued' });
		render(Plans);

		await screen.findByText('nightly-web');
		expect(screen.queryByText(/dispatcher/i)).not.toBeInTheDocument();
		await user.click(screen.getByRole('button', { name: 'More actions for nightly-web' }));
		// The menu content is hidden in jsdom, so the text query finds the item.
		await user.click(await screen.findByText('Run now'));

		await waitFor(() => expect(mocks.runPlan).toHaveBeenCalledWith('pl_1'));
		await waitFor(() =>
			expect(mocks.pushToast).toHaveBeenCalledWith('success', expect.stringContaining('queued'))
		);
	});

	it('hides every write action from a Viewer', async () => {
		session.role = 'viewer';
		mocks.listPlans.mockResolvedValue([plan()]);
		render(Plans);

		await screen.findByText('nightly-web');
		expect(screen.queryByRole('button', { name: 'Add plan' })).not.toBeInTheDocument();
		expect(
			screen.queryByRole('button', { name: 'More actions for nightly-web' })
		).not.toBeInTheDocument();
	});
});
