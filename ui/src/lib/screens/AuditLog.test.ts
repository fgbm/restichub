/*
 * The audit log pages on the server with numbered pages and a page size (FR-UI-01, FR-AU-01).
 */
import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import AuditLog from './AuditLog.svelte';

const mocks = vi.hoisted(() => ({ listAudit: vi.fn(), listPlatformAudit: vi.fn() }));

const route = vi.hoisted(() => ({ url: new URL('http://localhost/audit') }));

vi.mock('$app/state', () => ({ page: route }));
vi.mock('$app/navigation', () => ({
	goto: (address: string) => {
		route.url = new URL(address, 'http://localhost');
	}
}));

vi.mock('#lib/api/index.ts', async (importOriginal) => {
	const actual = await importOriginal<typeof import('#lib/api/index.ts')>();
	return {
		...actual,
		service: {
			...actual.service,
			listAudit: mocks.listAudit,
			listPlatformAudit: mocks.listPlatformAudit
		}
	};
});

function record(index: number) {
	return {
		id: `a_${index}`,
		tenant_id: 't_1',
		occurred_at: '2026-10-15T08:00:00Z',
		actor_type: 'system',
		actor_id: null,
		source_ip: null,
		action: 'job_run',
		object_type: 'job',
		object_id: `j_${index}`,
		diff: null
	};
}

function page(from: number, size: number, total: number, capped = false) {
	const count = Math.max(0, Math.min(size, total - from));
	return {
		items: Array.from({ length: count }, (_, index) => record(from + index)),
		next_cursor: null,
		total,
		total_capped: capped
	};
}

describe('the audit log', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		route.url = new URL('http://localhost/audit');
	});

	it('reads a numbered page with an offset and has no load more button', async () => {
		const user = userEvent.setup();
		mocks.listAudit.mockImplementation(async (query: { offset: number; limit: number }) =>
			page(query.offset, query.limit, 120)
		);
		render(AuditLog);

		await waitFor(() => expect(screen.getByText('Showing 1 to 25 of 120')).toBeInTheDocument());
		expect(mocks.listAudit).toHaveBeenCalledWith(expect.objectContaining({ offset: 0, limit: 25 }));
		expect(screen.queryByRole('button', { name: 'Load more' })).not.toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Page 1' })).toHaveAttribute('aria-current', 'page');

		await user.click(screen.getByRole('button', { name: 'Page 3' }));
		await waitFor(() =>
			expect(mocks.listAudit).toHaveBeenLastCalledWith(
				expect.objectContaining({ offset: 50, limit: 25 })
			)
		);
		expect(route.url.searchParams.get('page')).toBe('3');
		await waitFor(() => expect(screen.getByText('Showing 51 to 75 of 120')).toBeInTheDocument());
	});

	it('changes the page size and returns to the first page', async () => {
		const user = userEvent.setup();
		route.url = new URL('http://localhost/audit?page=2');
		mocks.listAudit.mockImplementation(async (query: { offset: number; limit: number }) =>
			page(query.offset, query.limit, 120)
		);
		render(AuditLog);
		await waitFor(() => expect(screen.getByText('Showing 26 to 50 of 120')).toBeInTheDocument());

		await user.selectOptions(screen.getByRole('combobox', { name: 'Rows per page' }), '50');
		await waitFor(() =>
			expect(mocks.listAudit).toHaveBeenLastCalledWith(
				expect.objectContaining({ offset: 0, limit: 50 })
			)
		);
		expect(route.url.searchParams.get('size')).toBe('50');
		expect(route.url.searchParams.get('page')).toBeNull();
	});

	it('shows a capped total with a plus and a hint', async () => {
		mocks.listPlatformAudit.mockImplementation(async (query: { offset: number; limit: number }) =>
			page(query.offset, query.limit, 10_000, true)
		);
		render(AuditLog, { props: { scope: 'platform' } });

		await waitFor(() => expect(screen.getByText('Showing 1 to 25 of 10,000+')).toBeInTheDocument());
		expect(
			screen.getByText('More than 10,000 records match. Narrow the filters to see the rest.')
		).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Page 400' })).toBeInTheDocument();
	});
});
