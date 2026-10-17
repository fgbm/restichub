import { afterEach, describe, expect, it } from 'vitest';
import type { Agent } from '#lib/api/index.ts';
import { setLocale } from '#lib/i18n/index.ts';
import {
	alertPill,
	alertState,
	apiTokenState,
	hostState,
	iconOf,
	jobState,
	pillOf,
	planState,
	repositoryState,
	tenantState
} from './vocabulary';

const now = Date.parse('2026-10-16T09:00:00Z');
const day = 86_400_000;

function agent(patch: Partial<Agent>): Agent {
	return {
		status: 'active',
		online: true,
		first_connected_at: '2026-01-01T00:00:00Z',
		certificate_expires_at: '2027-01-01T00:00:00Z',
		version_status: 'current',
		...patch
	} as Agent;
}

afterEach(() => setLocale('en'));

describe('job states', () => {
	it('labels each status with a result, a tone and a Lucide icon', () => {
		setLocale('ru');
		const cases = [
			[{ status: 'queued' }, 'В очереди', 'neutral', 'clock'],
			[{ status: 'sent' }, 'Отправлено хосту', 'neutral', 'send-horizontal'],
			[{ status: 'running' }, 'Выполняется', 'running', 'loader-circle'],
			[{ status: 'succeeded' }, 'Успешно', 'success', 'check-circle'],
			[{ status: 'failed' }, 'Не выполнено', 'danger', 'x-circle'],
			[{ status: 'failed', result_code: 'cancelled' }, 'Отменено', 'neutral', 'circle-slash'],
			[{ status: 'expired' }, 'Срок истёк', 'warning', 'clock-alert'],
			[{ status: 'dead_letter' }, 'Попытки исчерпаны', 'danger', 'octagon-x']
		] as const;
		for (const [job, label, tone, icon] of cases) {
			expect(jobState(job)).toMatchObject({ label, tone, icon });
		}
	});

	it('shows success with warnings as Warnings in the warning tone with the full meaning', () => {
		const state = jobState({ status: 'succeeded', result_code: 'succeeded_with_warnings' });
		expect(pillOf(state)).toEqual({
			tone: 'warning',
			label: 'Warnings',
			full: 'Succeeded with warnings'
		});
	});

	it('gives a success no pill and a green icon', () => {
		const state = jobState({ status: 'succeeded' });
		expect(pillOf(state)).toBeNull();
		expect(iconOf(state).tone).toBe('success');
	});

	it('spins only the running icon', () => {
		expect(iconOf(jobState({ status: 'running' })).spin).toBe(true);
		expect(iconOf(jobState({ status: 'queued' })).spin).toBeFalsy();
	});
});

describe('host states', () => {
	it('shows one pill by priority', () => {
		const soon = new Date(now + day).toISOString();
		expect(hostState(agent({ status: 'revoked', online: false }), now).key).toBe('revoked');
		expect(hostState(agent({ first_connected_at: null, online: false }), now).key).toBe('waiting');
		expect(hostState(agent({ online: false, certificate_expires_at: soon }), now).key).toBe(
			'offline'
		);
		expect(hostState(agent({ certificate_expires_at: soon }), now).key).toBe('certExpiring');
		expect(hostState(agent({ certificate_expires_at: '2026-10-01T00:00:00Z' }), now)).toMatchObject(
			{ key: 'certExpired', tone: 'danger' }
		);
		expect(hostState(agent({ version_status: 'unsupported' }), now).key).toBe('outdated');
		expect(pillOf(hostState(agent({}), now))).toBeNull();
		expect(iconOf(hostState(agent({}), now)).tone).toBe('success');
	});

	it('fr_ui_01 shows the use of an old certificate in the danger tone above offline', () => {
		const copied = agent({ online: false, superseded_certificate_used_at: '2026-10-16T08:00:00Z' });
		expect(hostState(copied, now)).toMatchObject({ key: 'oldCertificate', tone: 'danger' });
		expect(iconOf(hostState(copied, now)).tone).toBe('danger');
		expect(pillOf(hostState(copied, now))?.label).toBe('Old certificate used');
		expect(
			hostState({ ...copied, status: 'revoked' }, now).key,
			'a revoked host stays revoked'
		).toBe('revoked');
	});
});

describe('other entity states', () => {
	it('puts Disabled of a plan in the neutral tone', () => {
		expect(planState({ enabled: false })).toMatchObject({ tone: 'neutral', icon: 'calendar-off' });
		expect(pillOf(planState({ enabled: true }))).toBeNull();
	});

	it('fr_ui_01 puts Disabled of a plan above its recovery point state', () => {
		expect(planState({ enabled: true, rpo_state: 'violated' })).toMatchObject({
			key: 'violated',
			tone: 'danger',
			icon: 'calendar-x'
		});
		expect(planState({ enabled: true, rpo_state: 'at_risk' })).toMatchObject({
			key: 'atRisk',
			tone: 'warning'
		});
		expect(planState({ enabled: false, rpo_state: 'violated' }).key).toBe('disabled');
		expect(pillOf(planState({ enabled: true, rpo_state: 'unknown' }))).toBeNull();
	});

	it('fr_ui_01 ranks a repository: not initialized, then a failed check, then a stale lock', () => {
		const failed = { finished_at: '2026-10-16T08:00:00Z', result: 'failed' } as const;
		expect(
			repositoryState({ initialized: false, lock_state: 'stale', last_check: failed }).key
		).toBe('notInitialized');
		expect(
			repositoryState({ initialized: true, lock_state: 'stale', last_check: failed })
		).toMatchObject({ key: 'checkFailed', tone: 'danger', icon: 'database-zap' });
		expect(repositoryState({ initialized: true, lock_state: 'stale' })).toMatchObject({
			key: 'staleLock',
			tone: 'warning',
			icon: 'lock-keyhole'
		});
		expect(pillOf(repositoryState({ initialized: true, lock_state: 'held' }))).toBeNull();
	});

	it('warns about an API token that expires within 14 days', () => {
		const in9 = new Date(now + 9 * day).toISOString();
		expect(apiTokenState({ expires_at: in9, revoked_at: null }, now).key).toBe('expiring');
		expect(apiTokenState({ expires_at: null, revoked_at: '2026-10-01T00:00:00Z' }, now).key).toBe(
			'revoked'
		);
		expect(apiTokenState({ expires_at: '2026-10-01T00:00:00Z' }, now).key).toBe('expired');
	});

	it('gives an open alert the colored icon of its kind and no pill', () => {
		const open = { kind: 'rpo_violated', status: 'open' } as const;
		expect(iconOf(alertState(open))).toMatchObject({ name: 'calendar-x', tone: 'danger' });
		expect(alertPill(open)).toBeNull();
		const closed = { kind: 'rpo_violated', status: 'resolved' } as const;
		expect(iconOf(alertState(closed)).tone).toBe('neutral');
		expect(alertPill(closed)).toMatchObject({ tone: 'neutral', label: 'Resolved' });
	});

	it('puts Suspended of an organization above No administrator', () => {
		expect(tenantState({ status: 'suspended', admin_state: 'none' }).key).toBe('suspended');
		expect(tenantState({ status: 'active', admin_state: 'none' }).tone).toBe('warning');
	});
});
