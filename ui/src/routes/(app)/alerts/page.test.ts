/*
 * The Alerts screen lists the open alerts, the resolved alerts and all alerts, and pages on the
 * server with numbered pages (FR-MN-03, AC-MN-02, FR-UI-01).
 */
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Alerts from './+page.svelte';

const mocks = vi.hoisted(() => ({ listAlerts: vi.fn(), goto: vi.fn() }));

const route = vi.hoisted(() => ({ url: new URL('http://localhost/alerts') }));

vi.mock('$app/state', () => ({ page: route }));
vi.mock('$app/navigation', () => ({
	goto: (address: string) => {
		mocks.goto(address);
		route.url = new URL(address, 'http://localhost');
	}
}));

vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return { ...actual, service: { ...actual.service, listAlerts: mocks.listAlerts } };
});

const PAST = '2026-10-15T08:00:00Z';

function alert(overrides: Record<string, unknown> = {}) {
	return {
		id: 'al_1',
		kind: 'job_failed',
		status: 'open',
		agent_id: 'a_1',
		agent_hostname: 'win-fs01',
		plan_id: 'pl_1',
		plan_name: 'nightly-web',
		repository_id: 'r_1',
		repository_name: 'primary-s3',
		job_id: 'j_9',
		params: { job_kind: 'backup', error_code: 'config_error' },
		error_text: 'compile_failed',
		url: 'https://backup.example.com/jobs/j_9',
		opened_at: PAST,
		resolved_at: null,
		...overrides
	};
}

function list(items: ReturnType<typeof alert>[], total = items.length) {
	return { items, next_cursor: null, total, total_capped: false };
}

describe('the Alerts screen', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		route.url = new URL('http://localhost/alerts');
	});

	it('lists an open alert and opens its details with the link to the job', async () => {
		const user = userEvent.setup();
		mocks.listAlerts.mockResolvedValue(list([alert()]));
		render(Alerts);

		await waitFor(() => expect(screen.getByText('nightly-web')).toBeInTheDocument());
		expect(mocks.listAlerts).toHaveBeenCalledWith({ status: 'open', offset: 0, limit: 25 });
		expect(screen.getByText('Job failed')).toBeInTheDocument();
		expect(screen.getByText('The backup job failed (config_error).')).toBeInTheDocument();
		// The error text is in the details, not in the list (design/style/ui-principles.md, 2.5).
		expect(screen.queryByText('compile_failed')).not.toBeInTheDocument();
		expect(screen.queryByRole('link', { name: /Open the job/ })).not.toBeInTheDocument();

		await user.click(screen.getByRole('button', { name: 'nightly-web' }));
		const panel = await screen.findByRole('dialog');
		expect(within(panel).getByText('compile_failed')).toBeInTheDocument();
		expect(within(panel).getByText('win-fs01')).toBeInTheDocument();
		expect(within(panel).getByText('primary-s3')).toBeInTheDocument();
		expect(within(panel).getByRole('link', { name: 'Open the job' })).toHaveAttribute(
			'href',
			'/jobs/j_9'
		);
	});

	it('shows the empty state of the open list', async () => {
		mocks.listAlerts.mockResolvedValue(list([]));
		render(Alerts);

		expect(await screen.findByText('No open alerts')).toBeInTheDocument();
	});

	it('asks for the resolved alerts when the filter changes', async () => {
		const user = userEvent.setup();
		mocks.listAlerts.mockResolvedValueOnce(list([alert()])).mockResolvedValueOnce(
			list([
				alert({
					id: 'al_0',
					kind: 'rpo_violated',
					status: 'resolved',
					job_id: null,
					params: { elapsed_seconds: 31 * 3600, rpo_seconds: 26 * 3600 },
					error_text: null,
					resolved_at: '2026-10-15T09:00:00Z'
				})
			])
		);
		render(Alerts);
		await screen.findByText('nightly-web');

		const bar = screen.getByRole('group', { name: 'Show alerts' });
		await user.click(within(bar).getByRole('button', { name: /^State/ }));
		await user.click((await screen.findByText('Resolved')).closest('button') as HTMLElement);

		await waitFor(() =>
			expect(mocks.listAlerts).toHaveBeenLastCalledWith({
				status: 'resolved',
				offset: 0,
				limit: 25
			})
		);
		const row = (await screen.findByText('Backup overdue')).closest('tr') as HTMLElement;
		expect(within(row).getByText('Resolved')).toBeInTheDocument();
	});

	it('reads another numbered page and filters the kind on the server', async () => {
		const user = userEvent.setup();
		mocks.listAlerts.mockImplementation(async (query: { offset: number }) =>
			list(
				Array.from({ length: 25 }, (_, index) =>
					alert({ id: `al_${query.offset + index}`, plan_name: `plan-${query.offset + index}` })
				),
				60
			)
		);
		render(Alerts);
		await screen.findByText('plan-0');
		expect(screen.queryByRole('button', { name: 'Load more' })).not.toBeInTheDocument();
		expect(screen.getByText('Showing 1 to 25 of 60')).toBeInTheDocument();

		await user.click(screen.getByRole('button', { name: 'Page 3' }));
		await waitFor(() =>
			expect(mocks.listAlerts).toHaveBeenLastCalledWith({ status: 'open', offset: 50, limit: 25 })
		);
		expect(await screen.findByText('plan-50')).toBeInTheDocument();

		const bar = screen.getByRole('group', { name: 'Show alerts' });
		await user.click(within(bar).getByRole('button', { name: /^Alert/ }));
		await user.click(await screen.findByLabelText('Host offline'));
		await waitFor(() =>
			expect(mocks.listAlerts).toHaveBeenLastCalledWith({
				status: 'open',
				kind: 'agent_offline',
				offset: 0,
				limit: 25
			})
		);
	});
});
