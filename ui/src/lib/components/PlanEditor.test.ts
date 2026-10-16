/*
 * The plan page: create, edit, the server validation, the next runs and the read-only state of a
 * Viewer (FR-SC-01, FR-SC-07, ADR-0036).
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '#lib/api/index.ts';
import PlanEditor from './PlanEditor.svelte';

const mocks = vi.hoisted(() => ({
	listPlans: vi.fn(),
	listProfiles: vi.fn(),
	listAgents: vi.fn(),
	listRepositories: vi.fn(),
	createPlan: vi.fn(),
	updatePlan: vi.fn(),
	deletePlan: vi.fn(),
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
			listPlans: mocks.listPlans,
			listProfiles: mocks.listProfiles,
			listAgents: mocks.listAgents,
			listRepositories: mocks.listRepositories,
			createPlan: mocks.createPlan,
			updatePlan: mocks.updatePlan,
			deletePlan: mocks.deletePlan,
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

const input = (selector: string) =>
	document.querySelector<HTMLInputElement>(selector) as HTMLInputElement;

describe('the plan page', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		session.role = 'admin';
		mocks.listPlans.mockResolvedValue([plan()]);
		mocks.listProfiles.mockResolvedValue([
			{ id: 'p_1', name: 'web-daily', repository_id: 'r_1' },
			{ id: 'p_2', name: 'db-hourly', repository_id: 'r_1' }
		]);
		mocks.listAgents.mockResolvedValue([
			{ id: 'a_1', hostname: 'win-fs01', status: 'active' },
			{ id: 'a_2', hostname: 'old-host', status: 'revoked' }
		]);
		mocks.listRepositories.mockResolvedValue([{ id: 'r_1', name: 'repo-a' }]);
		mocks.previewSchedule.mockResolvedValue({ time_zone: 'Europe/Berlin', runs: [] });
	});

	it('fr_sc_01 creates a plan and returns to the list', async () => {
		const user = userEvent.setup();
		mocks.createPlan.mockResolvedValue({ plan: plan() });
		render(PlanEditor);

		expect(await screen.findByRole('heading', { level: 1, name: 'New plan' })).toBeInTheDocument();
		await user.type(input('#plan-name'), 'nightly-web');
		await user.type(input('#plan-cron'), '0 2 * * *');
		// A new plan starts with the browser zone, so the test replaces it.
		await user.clear(input('#plan-time-zone'));
		await user.type(input('#plan-time-zone'), 'Europe/Berlin');
		await user.click(screen.getByRole('button', { name: 'Save' }));

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
		expect(mocks.pushToast).toHaveBeenCalledWith('success', 'The plan is created.');
		expect(mocks.goto).toHaveBeenCalledWith('/(app)/plans');
	});

	it('fr_sc_01 offers only the active hosts', async () => {
		render(PlanEditor);

		const host = await screen.findByLabelText(/Host/, { selector: 'select' });
		expect(within(host).getByRole('option', { name: 'win-fs01' })).toBeInTheDocument();
		expect(within(host).queryByRole('option', { name: 'old-host' })).not.toBeInTheDocument();
	});

	it('fr_sc_01 edits a stored plan', async () => {
		const user = userEvent.setup();
		mocks.updatePlan.mockResolvedValue({ plan: plan() });
		render(PlanEditor, { planId: 'pl_1' });

		expect(
			await screen.findByRole('heading', { level: 1, name: 'nightly-web' })
		).toBeInTheDocument();
		expect(input('#plan-cron').value).toBe('0 2 * * *');
		expect(screen.getAllByText('repo-a').length).toBeGreaterThan(0);
		await user.selectOptions(input('#plan-profile'), 'p_2');
		await user.click(screen.getByRole('button', { name: 'Save' }));

		await waitFor(() =>
			expect(mocks.updatePlan).toHaveBeenCalledWith('pl_1', {
				name: 'nightly-web',
				profile_id: 'p_2',
				agent_id: 'a_1',
				cron: '0 2 * * *',
				time_zone: 'Europe/Berlin',
				missed_run_policy: 'latest_only',
				enabled: true,
				rpo_seconds: null
			})
		);
		expect(mocks.pushToast).toHaveBeenCalledWith('success', 'The plan is updated.');
	});

	it('deletes a stored plan after the confirmation and returns to the list', async () => {
		const user = userEvent.setup();
		mocks.deletePlan.mockResolvedValue(undefined);
		render(PlanEditor, { planId: 'pl_1' });

		await screen.findByRole('heading', { level: 1, name: 'nightly-web' });
		await user.click(screen.getByRole('button', { name: 'Delete plan' }));
		const dialog = await screen.findByRole('dialog');
		await user.click(within(dialog).getByRole('button', { name: 'Delete plan' }));

		await waitFor(() => expect(mocks.deletePlan).toHaveBeenCalledWith('pl_1'));
		expect(mocks.goto).toHaveBeenCalledWith('/(app)/plans');
	});

	it('shows an error for a plan that does not exist', async () => {
		render(PlanEditor, { planId: 'pl_missing' });

		expect(await screen.findByText('The plan does not exist.')).toBeInTheDocument();
	});

	it('adr_0036 sends the objective of the plan in seconds', async () => {
		const user = userEvent.setup();
		mocks.updatePlan.mockResolvedValue({ plan: plan() });
		render(PlanEditor, { planId: 'pl_1' });

		await screen.findByRole('heading', { level: 1, name: 'nightly-web' });
		await user.type(input('#plan-rpo'), '24,5');
		await user.click(screen.getByRole('button', { name: 'Save' }));

		await waitFor(() =>
			expect(mocks.updatePlan).toHaveBeenCalledWith(
				'pl_1',
				expect.objectContaining({ rpo_seconds: 88200 })
			)
		);
	});

	it('fr_sc_01 shows the field that the control plane refuses and stays on the page', async () => {
		const user = userEvent.setup();
		mocks.updatePlan.mockRejectedValue(
			new ApiError({
				status: 400,
				code: 'invalid_argument',
				title: 'Bad Request',
				field: 'cron',
				reason: 'invalid_format'
			})
		);
		render(PlanEditor, { planId: 'pl_1' });

		await screen.findByRole('heading', { level: 1, name: 'nightly-web' });
		// A collapsed section opens when the server names its field.
		await user.click(screen.getByRole('button', { name: 'Collapse all' }));
		await user.click(screen.getByRole('button', { name: 'Save' }));

		expect(await screen.findByText('The format is not valid.')).toBeInTheDocument();
		expect(input('#plan-cron')).toBeVisible();
		expect(mocks.goto).not.toHaveBeenCalled();
	});

	it('fr_sc_01 previews the next runs of the form schedule with UTC in the tooltip', async () => {
		const user = userEvent.setup();
		mocks.previewSchedule.mockResolvedValue({
			time_zone: 'UTC',
			runs: ['2999-01-01T02:00:00Z', '2999-01-02T02:00:00Z']
		});
		render(PlanEditor, { planId: 'pl_1' });

		await screen.findByRole('heading', { level: 1, name: 'nightly-web' });
		await user.clear(input('#plan-time-zone'));
		await user.type(input('#plan-time-zone'), 'UTC');

		await waitFor(() =>
			expect(mocks.previewSchedule).toHaveBeenLastCalledWith({
				cron: '0 2 * * *',
				time_zone: 'UTC',
				count: 5
			})
		);
		const runs = await waitFor(() => {
			const items = document.querySelectorAll('.runs li');
			expect(items).toHaveLength(2);
			return items;
		});
		expect(runs[0].getAttribute('title')).toMatch(/UTC/);
	});

	it('lets a Viewer read the plan without a save button', async () => {
		session.role = 'viewer';
		render(PlanEditor, { planId: 'pl_1' });

		await screen.findByRole('heading', { level: 1, name: 'nightly-web' });
		expect(screen.getByText('Your role cannot change a plan.')).toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Save' })).not.toBeInTheDocument();
	});
});
