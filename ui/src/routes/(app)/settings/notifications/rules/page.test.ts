/*
 * The alert rule settings show one row for each alert kind, change a switch and a threshold, and
 * save the rules (FR-MN-03).
 */
import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Rules from './+page.svelte';

const mocks = vi.hoisted(() => ({
	listAlertRules: vi.fn(),
	updateAlertRules: vi.fn(),
	pushToast: vi.fn()
}));

const session = vi.hoisted(() => ({ role: 'admin' }));

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
			listAlertRules: mocks.listAlertRules,
			updateAlertRules: mocks.updateAlertRules
		}
	};
});

const RULES = [
	{ kind: 'rpo_violated', enabled: true, threshold_minutes: null },
	{ kind: 'job_failed', enabled: true, threshold_minutes: null },
	{ kind: 'job_dead_letter', enabled: true, threshold_minutes: null },
	{ kind: 'job_warnings', enabled: false, threshold_minutes: null },
	{ kind: 'agent_offline', enabled: true, threshold_minutes: 30 },
	{ kind: 'stale_lock', enabled: true, threshold_minutes: 60 }
];

describe('the alert rule settings', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		session.role = 'admin';
		mocks.listAlertRules.mockResolvedValue(RULES);
		mocks.updateAlertRules.mockImplementation(
			async (request: { items: typeof RULES }) => request.items
		);
	});

	it('shows a row for each kind with the default thresholds', async () => {
		render(Rules);

		expect(await screen.findByText('Backup overdue')).toBeInTheDocument();
		expect(screen.getByText('Job with warnings')).toBeInTheDocument();
		expect(document.querySelector<HTMLInputElement>('#rule-agent_offline-minutes')?.value).toBe(
			'30'
		);
		expect(document.querySelector<HTMLInputElement>('#rule-stale_lock-minutes')?.value).toBe('60');
		expect(document.querySelector('#rule-job_warnings')).toHaveAttribute('aria-checked', 'false');
	});

	it('saves a switched rule and a new threshold', async () => {
		const user = userEvent.setup();
		render(Rules);
		await screen.findByText('Backup overdue');

		await user.click(document.querySelector('#rule-job_warnings') as HTMLElement);
		const minutes = document.querySelector<HTMLInputElement>('#rule-agent_offline-minutes');
		await user.clear(minutes as HTMLInputElement);
		await user.type(minutes as HTMLInputElement, '45');
		await user.click(screen.getByRole('button', { name: 'Save rules' }));

		await waitFor(() => expect(mocks.updateAlertRules).toHaveBeenCalledTimes(1));
		const { items } = mocks.updateAlertRules.mock.calls[0][0];
		expect(items).toHaveLength(6);
		expect(items.find((item: { kind: string }) => item.kind === 'job_warnings').enabled).toBe(true);
		expect(items.find((item: { kind: string }) => item.kind === 'agent_offline')).toEqual({
			kind: 'agent_offline',
			enabled: true,
			threshold_minutes: 45
		});
		expect(items.find((item: { kind: string }) => item.kind === 'job_failed')).toEqual({
			kind: 'job_failed',
			enabled: true,
			threshold_minutes: null
		});
		expect(mocks.pushToast).toHaveBeenCalledWith('success', 'The rules are saved.');
	});

	// The UI hides what the API refuses: an Admin changes the rules (ADR-0036, AC-AU-02).
	it.each(['operator', 'viewer'] as const)(
		'ac_au_02_shows_the_rules_to_a_%s_without_a_way_to_change_them',
		async (role) => {
			const user = userEvent.setup();
			session.role = role;
			render(Rules);

			await screen.findByText('Backup overdue');
			expect(screen.queryByRole('button', { name: 'Save rules' })).toBeNull();
			const send = document.querySelector<HTMLElement>('#rule-job_warnings') as HTMLElement;
			expect(send).toBeDisabled();
			expect(document.querySelector('#rule-agent_offline-minutes')).toBeDisabled();
			await user.click(send);
			expect(send).toHaveAttribute('aria-checked', 'false');
		}
	);
});
