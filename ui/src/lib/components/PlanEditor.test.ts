/*
 * The plan page: create, edit, the server validation, the next runs and the read-only state of a
 * Viewer (FR-SC-01, FR-SC-07, ADR-0036).
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '#lib/api/index.ts';
import { setLocale } from '#lib/i18n/index.ts';

afterEach(() => setLocale('en'));
import PlanEditor from './PlanEditor.svelte';

const mocks = vi.hoisted(() => ({
	listPlans: vi.fn(),
	listProfiles: vi.fn(),
	listAgents: vi.fn(),
	listRepositories: vi.fn(),
	listSecrets: vi.fn(),
	getAgentParameters: vi.fn(),
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
			listSecrets: mocks.listSecrets,
			getAgentParameters: mocks.getAgentParameters,
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
		parameter_values: [],
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

function agent(id: string, hostname: string, status = 'active') {
	return {
		id,
		hostname,
		status,
		os: 'windows',
		arch: 'amd64',
		labels: [],
		online: true,
		first_connected_at: FUTURE,
		last_seen_at: FUTURE
	};
}

const input = (selector: string) =>
	document.querySelector<HTMLInputElement>(selector) as HTMLInputElement;

const optionOf = (name: string) =>
	screen.getByText(name, { selector: '.rh-record__option-label' }).closest('[role="option"]')!;
const hostOptions = () =>
	[...document.querySelectorAll('[role="option"]')].map(
		(option) => option.querySelector('.rh-record__option-label')?.textContent?.trim() ?? ''
	);

async function pickProfile(user: ReturnType<typeof userEvent.setup>, name: string): Promise<void> {
	await screen.findByRole('heading', { level: 1 });
	await user.click(input('#plan-profile'));
	const label = await screen.findByText(name, { selector: '.rh-record__option-label' });
	await user.click(label.closest('[role="option"]')!);
}

async function pickRepository(
	user: ReturnType<typeof userEvent.setup>,
	name: string
): Promise<void> {
	await screen.findByRole('heading', { level: 1 });
	await user.click(input('#plan-repository'));
	const label = await screen.findByText(name, { selector: '.rh-record__option-label' });
	await user.click(label.closest('[role="option"]')!);
}

async function openHosts(user: ReturnType<typeof userEvent.setup>): Promise<HTMLElement> {
	const host = await waitFor(() => {
		const field = input('#plan-agent');
		expect(field).toBeTruthy();
		return field;
	});
	await user.click(host);
	await screen.findAllByRole('option', { hidden: true });
	return host;
}

describe('the plan page', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		session.role = 'admin';
		mocks.listPlans.mockResolvedValue([plan()]);
		mocks.listProfiles.mockResolvedValue([
			{ id: 'p_1', name: 'web-daily', paths: { all: ['/srv'] }, retention: null },
			{ id: 'p_2', name: 'db-hourly', paths: { all: ['/var/backups'] }, retention: null }
		]);
		mocks.listAgents.mockResolvedValue([
			agent('a_1', 'win-fs01'),
			agent('a_2', 'old-host', 'revoked')
		]);
		mocks.listRepositories.mockResolvedValue([
			{
				id: 'r_1',
				name: 'repo-a',
				backend: { kind: 'rest', endpoint: 'https://rest.example.net/a' },
				maintenance_agent_id: 'a_1',
				initialized: true
			}
		]);
		mocks.previewSchedule.mockResolvedValue({ time_zone: 'Europe/Berlin', runs: [] });
		mocks.listSecrets.mockResolvedValue([]);
		mocks.getAgentParameters.mockResolvedValue({ agent_id: 'a_1', parameters: [] });
	});

	it('fr_sc_01 creates a plan and returns to the list', async () => {
		const user = userEvent.setup();
		mocks.createPlan.mockResolvedValue({ plan: plan() });
		render(PlanEditor);

		expect(await screen.findByRole('heading', { level: 1, name: 'New plan' })).toBeInTheDocument();
		await user.clear(input('#plan-name'));
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
				repository_id: 'r_1',
				agent_id: 'a_1',
				cron: '0 2 * * *',
				time_zone: 'Europe/Berlin',
				missed_run_policy: 'run_all',
				enabled: true,
				rpo_seconds: null,
				parameter_values: []
			})
		);
		expect(mocks.pushToast).toHaveBeenCalledWith('success', 'The plan is created.');
		expect(mocks.goto).toHaveBeenCalledWith('/(app)/plans');
	});

	it('shows the sentence of the plans list on the page of a new plan and a stored plan', async () => {
		const sentence = 'A plan backs up a profile from a host to a repository on a schedule.';
		const first = render(PlanEditor);
		expect(await screen.findByText(sentence)).toBeInTheDocument();
		first.unmount();

		render(PlanEditor, { planId: 'pl_1' });
		await screen.findByRole('heading', { level: 1, name: 'nightly-web' });
		expect(screen.getByText(sentence)).toBeInTheDocument();
	});

	it('shows the effective objective with the unit of the locale', async () => {
		setLocale('ru');
		render(PlanEditor, { planId: 'pl_1' });
		await screen.findByRole('heading', { level: 1, name: 'nightly-web' });
		expect((await screen.findAllByText(/Действует: 26 ч/)).length).toBeGreaterThan(0);
		expect(screen.queryByText(/26 h/)).not.toBeInTheDocument();
	});

	it('adr_0047 shows the profile and the repository of a stored plan', async () => {
		const user = userEvent.setup();
		render(PlanEditor, { planId: 'pl_1' });

		await screen.findByRole('heading', { level: 1, name: 'nightly-web' });
		expect(input('#plan-profile')).toHaveTextContent(/web-daily\s*· \/srv/);
		expect(input('#plan-repository')).toHaveTextContent(
			/repo-a\s*· REST · https:\/\/rest.example.net\/a/
		);
		await user.click(input('#plan-repository'));
		expect(optionOf('repo-a')).toHaveTextContent('REST');
		expect(optionOf('repo-a')).toHaveTextContent('https://rest.example.net/a');
		expect(screen.getByText('1 repository')).toBeInTheDocument();
	});

	it('adr_0047 sends an empty repository and shows the refusal of the server', async () => {
		const user = userEvent.setup();
		mocks.listRepositories.mockResolvedValue([]);
		mocks.createPlan.mockRejectedValue(
			new ApiError({
				status: 400,
				code: 'invalid_argument',
				title: 'Bad Request',
				field: 'repository_id',
				reason: 'empty'
			})
		);
		render(PlanEditor);

		await screen.findByRole('heading', { level: 1, name: 'New plan' });
		await user.clear(input('#plan-name'));
		await user.type(input('#plan-name'), 'nightly-web');
		await user.type(input('#plan-cron'), '0 2 * * *');
		await user.click(screen.getByRole('button', { name: 'Save' }));

		await waitFor(() =>
			expect(mocks.createPlan).toHaveBeenCalledWith(expect.objectContaining({ repository_id: '' }))
		);
		expect(await screen.findByText('This field is required.')).toBeInTheDocument();
		expect(input('#plan-repository')).toHaveAttribute('aria-invalid', 'true');
	});

	it('fr_sc_01 offers only the active hosts with their state and platform', async () => {
		const user = userEvent.setup();
		render(PlanEditor);

		const host = await openHosts(user);
		expect(host).toHaveTextContent(/win-fs01\s*· Windows · Online/);
		expect(hostOptions()).toEqual(['win-fs01']);
		expect(optionOf('win-fs01')).toHaveTextContent(/Windows · amd64/);
		expect(screen.getByText('1 host')).toBeInTheDocument();
	});

	it('ac_rp_07 offers only the repository host for a local repository', async () => {
		mocks.listAgents.mockResolvedValue([agent('a_1', 'win-fs01'), agent('a_3', 'nas-01')]);
		mocks.listRepositories.mockResolvedValue([
			{
				id: 'r_1',
				name: 'repo-a',
				backend: { kind: 'rest', endpoint: 'https://rest.example.net/a' },
				maintenance_agent_id: 'a_1',
				initialized: true
			},
			{
				id: 'r_2',
				name: 'nas-local',
				backend: { kind: 'local', path: '/srv/restic' },
				maintenance_agent_id: 'a_3',
				initialized: true
			}
		]);
		const user = userEvent.setup();
		render(PlanEditor);

		await pickRepository(user, 'nas-local');
		const host = await openHosts(user);
		await waitFor(() => expect(host).toHaveTextContent('nas-01'));
		expect(hostOptions()).toEqual(['win-fs01', 'nas-01']);
		const other = optionOf('win-fs01');
		expect(other).toHaveAttribute('data-disabled');
		expect(other).toHaveAccessibleDescription(/Repository nas-local is on nas-01/);
		expect(optionOf('nas-01')).not.toHaveAttribute('data-disabled');
		expect(screen.getByText(/Its path is on the host nas-01 only/)).toBeInTheDocument();
	});

	it('ac_rp_07 keeps the host of a stored plan on another host and shows the error', async () => {
		mocks.listAgents.mockResolvedValue([agent('a_1', 'win-fs01'), agent('a_3', 'nas-01')]);
		mocks.listRepositories.mockResolvedValue([
			{
				id: 'r_1',
				name: 'nas-local',
				backend: { kind: 'local', path: '/srv/restic' },
				maintenance_agent_id: 'a_3',
				initialized: true
			}
		]);
		const user = userEvent.setup();
		mocks.updatePlan.mockResolvedValue({ plan: plan() });
		render(PlanEditor, { planId: 'pl_1' });

		await screen.findByRole('heading', { level: 1, name: 'nightly-web' });
		const host = input('#plan-agent');
		expect(host).toHaveTextContent('win-fs01');
		expect(host).toHaveAttribute('aria-invalid', 'true');
		expect(screen.getByText(/Select the repository host/)).toBeInTheDocument();

		await user.clear(input('#plan-cron'));
		await user.type(input('#plan-cron'), '0 3 * * *');
		await user.click(screen.getByRole('button', { name: 'Save' }));
		await waitFor(() =>
			expect(mocks.updatePlan).toHaveBeenCalledWith(
				'pl_1',
				expect.objectContaining({ agent_id: 'a_1' })
			)
		);
	});

	it('ac_rp_07 clears the host when the local repository host is not active', async () => {
		mocks.listRepositories.mockResolvedValue([
			{
				id: 'r_1',
				name: 'repo-a',
				backend: { kind: 'rest', endpoint: 'https://rest.example.net/a' },
				maintenance_agent_id: 'a_1',
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
		const user = userEvent.setup();
		render(PlanEditor);

		await pickRepository(user, 'nas-local');
		const host = await openHosts(user);

		expect(host).not.toHaveTextContent('win-fs01');
		expect(optionOf('win-fs01')).toHaveAttribute('data-disabled');
		expect(screen.getByText(/Select the repository host/)).toBeInTheDocument();
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
		await pickProfile(user, 'db-hourly');
		await user.click(screen.getByRole('button', { name: 'Save' }));

		await waitFor(() =>
			expect(mocks.updatePlan).toHaveBeenCalledWith('pl_1', {
				name: 'nightly-web',
				profile_id: 'p_2',
				repository_id: 'r_1',
				agent_id: 'a_1',
				cron: '0 2 * * *',
				time_zone: 'Europe/Berlin',
				missed_run_policy: 'latest_only',
				enabled: true,
				rpo_seconds: null,
				parameter_values: []
			})
		);
		expect(mocks.pushToast).toHaveBeenCalledWith('success', 'The plan is updated.');
	});

	it('fr_sc_01 turns the plan off with the switch in the schedule header', async () => {
		const user = userEvent.setup();
		mocks.updatePlan.mockResolvedValue({ plan: plan({ enabled: false }) });
		render(PlanEditor, { planId: 'pl_1' });

		await screen.findByRole('heading', { level: 1, name: 'nightly-web' });
		expect(screen.queryByText('Disabled')).not.toBeInTheDocument();
		expect(screen.queryByRole('checkbox', { name: 'Enabled' })).not.toBeInTheDocument();
		const schedule = document.querySelector<HTMLElement>('[data-section="schedule"]')!;
		const toggle = within(schedule).getByRole('switch', { name: 'Run on schedule' });
		expect(toggle).toHaveAttribute('data-state', 'checked');
		expect(within(schedule).queryByText(/The scheduler does not run this plan/)).toBeNull();

		await user.click(toggle);

		expect(toggle).toHaveAttribute('data-state', 'unchecked');
		expect(within(schedule).getByText(/The scheduler does not run this plan/)).toHaveTextContent(
			/"Run now" still works/
		);
		expect(screen.getByRole('heading', { level: 1 }).parentElement).toHaveTextContent('Disabled');
		await user.click(screen.getByRole('button', { name: 'Save' }));
		await waitFor(() =>
			expect(mocks.updatePlan).toHaveBeenCalledWith(
				'pl_1',
				expect.objectContaining({ enabled: false })
			)
		);
	});

	it('fr_sc_01 names a new plan from the host and the profile and follows a change of the profile', async () => {
		const user = userEvent.setup();
		render(PlanEditor);

		await screen.findByRole('heading', { level: 1, name: 'New plan' });
		await waitFor(() => expect(input('#plan-name')).toHaveValue('win-fs01 | web-daily'));
		await pickProfile(user, 'db-hourly');
		await waitFor(() => expect(input('#plan-name')).toHaveValue('win-fs01 | db-hourly'));
	});

	it('fr_sc_01 makes the default name unique among the plans', async () => {
		mocks.listPlans.mockResolvedValue([plan({ name: 'WIN-FS01 | Web-Daily' })]);
		render(PlanEditor);

		await screen.findByRole('heading', { level: 1, name: 'New plan' });
		await waitFor(() => expect(input('#plan-name')).toHaveValue('win-fs01 | web-daily (2)'));
	});

	it('fr_sc_01 keeps the name that the user typed when the profile changes', async () => {
		const user = userEvent.setup();
		render(PlanEditor);

		await screen.findByRole('heading', { level: 1, name: 'New plan' });
		await waitFor(() => expect(input('#plan-name')).toHaveValue('win-fs01 | web-daily'));
		await user.type(input('#plan-name'), '!');
		await pickProfile(user, 'db-hourly');
		await waitFor(() => expect(input('#plan-profile')).toHaveTextContent('db-hourly'));
		expect(input('#plan-name')).toHaveValue('win-fs01 | web-daily!');
	});

	it('fr_sc_01 does not change the name of a stored plan', async () => {
		const user = userEvent.setup();
		render(PlanEditor, { planId: 'pl_1' });

		await screen.findByRole('heading', { level: 1, name: 'nightly-web' });
		await pickProfile(user, 'db-hourly');
		await waitFor(() => expect(input('#plan-profile')).toHaveTextContent('db-hourly'));
		expect(input('#plan-name')).toHaveValue('nightly-web');
	});

	it('fr_sc_01 shows the source section before the basics', async () => {
		render(PlanEditor);

		await screen.findByRole('heading', { level: 1, name: 'New plan' });
		const order = [...document.querySelectorAll('[data-section]')].map((node) =>
			node.getAttribute('data-section')
		);
		expect(order).toEqual(['source', 'parameters', 'basics', 'schedule', 'objective']);
	});

	it('keeps the delete of a stored plan in the dangerous section only', async () => {
		render(PlanEditor, { planId: 'pl_1' });
		await screen.findByRole('heading', { level: 1, name: 'nightly-web' });

		const danger = screen
			.getByRole('button', { name: /Dangerous actions/ })
			.closest('section') as HTMLElement;
		expect(danger).toHaveAttribute('data-section', 'danger');
		expect(
			within(danger).getByText(/The scheduler does not run the plan any more/)
		).toBeInTheDocument();
		expect(within(danger).getByRole('button', { name: 'Delete plan' })).toHaveClass('btn--danger');
		const header = document.querySelector<HTMLElement>('.page-header__actions') as HTMLElement;
		expect(within(header).queryByRole('button', { name: 'Delete plan' })).toBeNull();
	});

	it('shows no dangerous section for a new plan and for a viewer', async () => {
		const created = render(PlanEditor);
		await screen.findByRole('heading', { level: 1, name: 'New plan' });
		expect(document.querySelector('[data-section="danger"]')).toBeNull();
		created.unmount();

		session.role = 'viewer';
		render(PlanEditor, { planId: 'pl_1' });
		await screen.findByRole('heading', { level: 1, name: 'nightly-web' });
		expect(document.querySelector('[data-section="danger"]')).toBeNull();
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

	it('adr_0047_an_operator_deletes_no_plan_that_sets_a_secret_value', async () => {
		session.role = 'operator';
		mocks.listPlans.mockResolvedValue([
			plan({
				parameter_values: [
					{ name: 'pg_password', kind: 'secret', secret_id: 's_1', secret_name: 'pg' }
				]
			})
		]);
		render(PlanEditor, { planId: 'pl_1' });

		await screen.findByRole('heading', { level: 1, name: 'nightly-web' });
		const button = screen.getByRole('button', { name: 'Delete plan' });
		expect(button).toBeDisabled();
		expect(button).toHaveAttribute(
			'title',
			'Only an Admin deletes a plan that sets a secret value.'
		);
	});

	it('adr_0047_an_admin_deletes_a_plan_that_sets_a_secret_value', async () => {
		mocks.listPlans.mockResolvedValue([
			plan({
				parameter_values: [
					{ name: 'pg_password', kind: 'secret', secret_id: 's_1', secret_name: 'pg' }
				]
			})
		]);
		render(PlanEditor, { planId: 'pl_1' });

		await screen.findByRole('heading', { level: 1, name: 'nightly-web' });
		expect(screen.getByRole('button', { name: 'Delete plan' })).toBeEnabled();
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
		await user.type(input('#plan-name'), '-2');
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

	it('fr_sc_01 says there is no next run while the plan is off', async () => {
		const user = userEvent.setup();
		mocks.previewSchedule.mockResolvedValue({ time_zone: 'UTC', runs: ['2999-01-01T02:00:00Z'] });
		render(PlanEditor, { planId: 'pl_1' });

		await screen.findByRole('heading', { level: 1, name: 'nightly-web' });
		await user.click(screen.getByRole('switch', { name: 'Run on schedule' }));

		expect(await screen.findByText('None — the plan is off')).toBeInTheDocument();
	});

	it('lets a Viewer read the plan without a save button', async () => {
		session.role = 'viewer';
		render(PlanEditor, { planId: 'pl_1' });

		await screen.findByRole('heading', { level: 1, name: 'nightly-web' });
		expect(screen.getByText('Your role cannot change a plan.')).toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Save' })).not.toBeInTheDocument();
	});
});

describe('the parameters of the profile on the plan page (FR-PF-10, FR-PF-11, ADR-0047)', () => {
	function definition(overrides: Record<string, unknown> = {}) {
		return {
			name: 'pg_dir',
			kind: 'text',
			required: true,
			default_value: null,
			description: null,
			hidden: false,
			...overrides
		};
	}

	function hostRow(overrides: Record<string, unknown> = {}) {
		return {
			name: 'pg_host',
			kind: 'text',
			required: true,
			default_value: null,
			description: null,
			needed_by: ['db-hourly'],
			value: { kind: 'text', value: 'db.local', secret_id: null, secret_name: null },
			unused: false,
			hidden: false,
			...overrides
		};
	}

	beforeEach(() => {
		vi.clearAllMocks();
		session.role = 'admin';
		mocks.listAgents.mockResolvedValue([agent('a_1', 'win-fs01')]);
		mocks.listRepositories.mockResolvedValue([
			{
				id: 'r_1',
				name: 'repo-a',
				backend: { kind: 'rest', endpoint: 'https://rest.example.net/a' },
				maintenance_agent_id: 'a_1',
				initialized: true
			}
		]);
		mocks.previewSchedule.mockResolvedValue({ time_zone: 'Europe/Berlin', runs: [] });
		mocks.listSecrets.mockResolvedValue([]);
		mocks.listProfiles.mockResolvedValue([
			{
				id: 'p_1',
				name: 'web-daily',
				paths: { all: ['/srv'] },
				retention: null,
				parameters: [
					definition({ name: 'pg_dir', default_value: '/var/backups/pg' }),
					definition({ name: 'pg_host' }),
					definition({ name: 'keep_logs', required: false })
				]
			}
		]);
		mocks.listPlans.mockResolvedValue([
			plan({
				profile_id: 'p_1',
				agent_id: 'a_1',
				parameter_values: [
					{
						name: 'keep_logs',
						kind: 'text',
						value: '30',
						secret_id: null,
						secret_name: null,
						hidden: false
					}
				]
			})
		]);
		mocks.getAgentParameters.mockResolvedValue({ agent_id: 'a_1', parameters: [hostRow()] });
	});

	async function openParameters(user: ReturnType<typeof userEvent.setup>): Promise<void> {
		const button = await screen.findByRole('button', { name: /^Profile parameters/ });
		if (button.getAttribute('aria-expanded') !== 'true') await user.click(button);
	}

	it('ac_pf_08_shows_the_value_of_the_plan_the_host_and_the_default', async () => {
		const user = userEvent.setup();
		render(PlanEditor, { planId: 'pl_1' });
		await screen.findByRole('heading', { level: 1, name: 'nightly-web' });
		await openParameters(user);

		expect(await screen.findByText('db.local')).toBeInTheDocument();
		expect(screen.getByText('/var/backups/pg')).toBeInTheDocument();
		expect(screen.getByText('30')).toBeInTheDocument();
		expect(screen.getByText('default')).toBeInTheDocument();
		expect(screen.getByText('host')).toBeInTheDocument();
		expect(screen.getByText('plan')).toBeInTheDocument();
	});

	it('ac_pf_06_disables_the_save_and_names_the_parameter_without_a_value', async () => {
		const user = userEvent.setup();
		mocks.getAgentParameters.mockResolvedValue({ agent_id: 'a_1', parameters: [] });
		render(PlanEditor, { planId: 'pl_1' });
		await screen.findByRole('heading', { level: 1, name: 'nightly-web' });
		await openParameters(user);

		expect(
			await screen.findByText('The plan cannot be saved: the parameter pg_host has no value.')
		).toBeInTheDocument();
		const header = screen.getByRole('button', { name: /^Profile parameters/ });
		expect(header.querySelector('[data-state="warn"]')).not.toBeNull();
		expect(screen.getByText('not set')).toBeInTheDocument();
		const save = screen.getByRole('button', { name: 'Save' });
		expect(save).toBeDisabled();
		expect(save).toHaveAttribute(
			'title',
			'The plan cannot be saved: the parameter pg_host has no value'
		);
	});

	it('ac_pf_06_sets_a_value_for_the_plan_and_removes_it', async () => {
		const user = userEvent.setup();
		mocks.getAgentParameters.mockResolvedValue({ agent_id: 'a_1', parameters: [] });
		render(PlanEditor, { planId: 'pl_1' });
		await screen.findByRole('heading', { level: 1, name: 'nightly-web' });
		await openParameters(user);

		await user.click(
			screen.getByRole('button', { name: 'Set the value of pg_host for this plan' })
		);
		await user.type(await screen.findByLabelText('Value'), 'plan.local');
		await user.click(screen.getByRole('button', { name: 'Set' }));

		expect(screen.getByText('plan.local')).toBeInTheDocument();
		await waitFor(() => expect(screen.getByRole('button', { name: 'Save' })).toBeEnabled());

		await user.click(
			screen.getByRole('button', { name: 'Remove the value of pg_host from the plan' })
		);
		expect(screen.queryByText('plan.local')).not.toBeInTheDocument();
	});

	it('ac_pf_08_opens_the_host_page_for_a_host_value', async () => {
		const user = userEvent.setup();
		render(PlanEditor, { planId: 'pl_1' });
		await screen.findByRole('heading', { level: 1, name: 'nightly-web' });
		await openParameters(user);

		await user.click(screen.getByRole('button', { name: 'Set the value of pg_host on the host' }));
		expect(mocks.goto).toHaveBeenCalledWith('/(app)/hosts/a_1#parameters');
	});
});
