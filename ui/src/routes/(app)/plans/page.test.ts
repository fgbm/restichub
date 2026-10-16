/*
 * The Plans screen lists the plans of the organization, links each plan to its page, runs one
 * now and deletes one (FR-SC-01, FR-SC-07, FR-SC-09).
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
	deletePlan: vi.fn(),
	runPlan: vi.fn(),
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
			deletePlan: mocks.deletePlan,
			runPlan: mocks.runPlan
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

describe('the Plans screen', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		session.role = 'admin';
		mocks.listPlans.mockResolvedValue([]);
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

	it('deletes a plan after the confirmation', async () => {
		const user = userEvent.setup();
		mocks.listPlans.mockResolvedValueOnce([plan()]).mockResolvedValueOnce([]);
		mocks.deletePlan.mockResolvedValue(undefined);
		render(Plans);

		await screen.findByText('nightly-web');
		await user.click(screen.getByRole('button', { name: 'Delete plan' }));
		const dialog = await screen.findByRole('dialog');
		await user.click(within(dialog).getByRole('button', { name: 'Delete plan' }));

		await waitFor(() => expect(mocks.deletePlan).toHaveBeenCalledWith('pl_1'));
		expect(await screen.findByText('No plans yet')).toBeInTheDocument();
	});
});
