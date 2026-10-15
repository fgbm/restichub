/*
 * The Overview screen shows the dashboard of the organization: the hosts, the plans with the RPO
 * state, the repository locks, the failed jobs and the open alerts (FR-MN-01, AC-MN-09). An
 * organization with nothing shows the first steps.
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Overview from './+page.svelte';

const mocks = vi.hoisted(() => ({ getDashboard: vi.fn() }));

vi.mock('#lib/stores/session.svelte.ts', () => ({
	getSession: () => ({
		user: { id: 'u_01', username: 'admin', display_name: 'Anna Petrova', role: 'admin' },
		tenant: { id: 't_01', name: 'acme' },
		role: 'admin'
	})
}));

vi.mock('#lib/stores/instance.svelte.ts', () => ({ isSingleTenant: () => false }));

vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return { ...actual, service: { ...actual.service, getDashboard: mocks.getDashboard } };
});

const PAST = '2026-10-14T08:00:00Z';

function dashboard(overrides: Record<string, unknown> = {}) {
	return {
		generated_at: PAST,
		hosts: {
			total: 2,
			online: 1,
			offline: 1,
			items: [
				{ id: 'a_2', hostname: 'mac-build-01', online: false, last_seen_at: PAST },
				{ id: 'a_1', hostname: 'win-fs01', online: true, last_seen_at: PAST }
			]
		},
		plans: {
			total: 3,
			at_risk: 1,
			violated: 1,
			items: [
				{
					id: 'pl_3',
					name: 'broken',
					agent_hostname: 'win-fs01',
					repository_name: 'primary-s3',
					enabled: true,
					last_success_at: null,
					age_seconds: 400000,
					rpo_seconds: 93600,
					state: 'violated'
				},
				{
					id: 'pl_2',
					name: 'risky',
					agent_hostname: 'win-fs01',
					repository_name: 'primary-s3',
					enabled: true,
					last_success_at: PAST,
					age_seconds: 75000,
					rpo_seconds: 93600,
					state: 'at_risk'
				},
				{
					id: 'pl_1',
					name: 'healthy',
					agent_hostname: 'win-fs01',
					repository_name: 'primary-s3',
					enabled: true,
					last_success_at: PAST,
					age_seconds: 3600,
					rpo_seconds: 93600,
					state: 'ok'
				}
			]
		},
		repositories: [
			{ id: 'r_1', name: 'primary-s3', lock_state: 'stale' },
			{ id: 'r_2', name: 'archive', lock_state: 'free' }
		],
		failed_jobs: [
			{
				id: 'j_9',
				kind: 'backup',
				status: 'failed',
				result_code: 'failed',
				agent_hostname: 'win-fs01',
				plan_name: 'broken',
				repository_name: 'primary-s3',
				finished_at: PAST
			}
		],
		alerts: {
			total: 1,
			items: [
				{
					id: 'al_1',
					kind: 'rpo_violated',
					status: 'open',
					agent_id: 'a_1',
					agent_hostname: 'win-fs01',
					plan_id: 'pl_3',
					plan_name: 'broken',
					repository_id: 'r_1',
					repository_name: 'primary-s3',
					job_id: null,
					params: { elapsed_seconds: 40 * 3600, rpo_seconds: 26 * 3600 },
					error_text: null,
					url: null,
					opened_at: PAST,
					resolved_at: null
				}
			]
		},
		...overrides
	};
}

describe('the Overview screen', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it('shows the counters, the plan states, the locks, the failed jobs and the alerts', async () => {
		mocks.getDashboard.mockResolvedValue(dashboard());
		render(Overview);

		await screen.findByText('Hosts online');
		const stats = document.querySelector('.stats') as HTMLElement;
		expect(within(stats).getByText('Hosts offline').previousElementSibling).toHaveTextContent('1');
		expect(within(stats).getByText('RPO violated').previousElementSibling).toHaveTextContent('1');
		expect(within(stats).getByText('Plans at risk').previousElementSibling).toHaveTextContent('1');

		const brokenRow = screen.getAllByText('broken')[0].closest('tr') as HTMLElement;
		expect(within(brokenRow).getByText('Violated')).toBeInTheDocument();
		expect(within(brokenRow).getByText('No successful backup yet')).toBeInTheDocument();
		const riskyRow = screen.getByText('risky').closest('tr') as HTMLElement;
		expect(within(riskyRow).getByText('At risk')).toBeInTheDocument();
		expect(
			within(screen.getByText('healthy').closest('tr') as HTMLElement).getByText('OK')
		).toBeInTheDocument();

		expect(screen.getByText('Stale lock')).toBeInTheDocument();
		expect(screen.getByText('No lock')).toBeInTheDocument();
		expect(screen.getByText('mac-build-01')).toBeInTheDocument();
		expect(screen.getByText('Backup overdue')).toBeInTheDocument();
		expect(screen.getByRole('link', { name: 'broken' })).toHaveAttribute('href', '/jobs/j_9');
	});

	it('shows the first steps for an organization with nothing', async () => {
		mocks.getDashboard.mockResolvedValue(
			dashboard({
				hosts: { total: 0, online: 0, offline: 0, items: [] },
				plans: { total: 0, at_risk: 0, violated: 0, items: [] },
				repositories: [],
				failed_jobs: [],
				alerts: { total: 0, items: [] }
			})
		);
		render(Overview);

		await waitFor(() => expect(screen.getByText('Get started')).toBeInTheDocument());
		expect(screen.queryByText('Hosts online')).not.toBeInTheDocument();
	});
});
