import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import type { Component } from 'svelte';
import { describe, expect, it } from 'vitest';
import DataTable from './DataTable.svelte';
import type { Column } from './types';

interface Row {
	id: string;
	name: string;
	count: number;
}

interface TableProps {
	columns: Column<Row>[];
	rows: Row[];
	rowKey: (row: Row) => string;
	pageSize?: number;
	loading?: boolean;
	error?: string | null;
	emptyTitle?: string;
}

// The generic component is used with one row type in these tests.
const Table = DataTable as unknown as Component<TableProps>;

const columns: Column<Row>[] = [
	{ key: 'name', header: 'Name' },
	{ key: 'count', header: 'Count', mono: true }
];

function makeRows(count: number): Row[] {
	return Array.from({ length: count }, (_, index) => ({
		id: `r_${index}`,
		name: `row ${String(index).padStart(2, '0')}`,
		count: count - index
	}));
}

function rowCells(container: HTMLElement): string[][] {
	return Array.from(container.querySelectorAll('tbody tr')).map((row) =>
		Array.from(row.querySelectorAll('td')).map((cell) => cell.textContent?.trim() ?? '')
	);
}

describe('DataTable', () => {
	it('renders one page of rows and moves to the next page', async () => {
		const user = userEvent.setup();
		const { container } = render(Table, {
			props: { columns, rows: makeRows(12), rowKey: (row: Row) => row.id, pageSize: 5 }
		});

		expect(container.querySelectorAll('tbody tr')).toHaveLength(5);
		expect(screen.getByText('Showing 1 to 5 of 12')).toBeInTheDocument();

		await user.click(screen.getByRole('button', { name: 'Next' }));
		expect(container.querySelectorAll('tbody tr')).toHaveLength(5);
		expect(rowCells(container)[0][0]).toBe('row 05');
	});

	it('filters rows with the client-side filter box', async () => {
		const user = userEvent.setup();
		const { container } = render(Table, {
			props: { columns, rows: makeRows(12), rowKey: (row: Row) => row.id, pageSize: 20 }
		});

		await user.type(screen.getByRole('searchbox'), 'row 03');
		expect(container.querySelectorAll('tbody tr')).toHaveLength(1);
		expect(rowCells(container)[0][0]).toBe('row 03');
	});

	it('sorts rows when a header is clicked', async () => {
		const user = userEvent.setup();
		const { container } = render(Table, {
			props: { columns, rows: makeRows(3), rowKey: (row: Row) => row.id, pageSize: 20 }
		});

		expect(rowCells(container)[0][1]).toBe('3');
		await user.click(screen.getByRole('button', { name: /Count/ }));
		expect(rowCells(container)[0][1]).toBe('1');
		await user.click(screen.getByRole('button', { name: /Count/ }));
		expect(rowCells(container)[0][1]).toBe('3');
	});

	it('shows skeleton rows while loading and an error row on failure', () => {
		const { container, rerender } = render(Table, {
			props: { columns, rows: [], rowKey: (row: Row) => row.id, loading: true, pageSize: 3 }
		});
		expect(container.querySelectorAll('[data-testid="skeleton-row"]')).toHaveLength(3);

		rerender({ columns, rows: [], rowKey: (row: Row) => row.id, loading: false, error: 'Boom' });
		expect(screen.getByRole('alert')).toHaveTextContent('Boom');
	});

	it('shows the empty state when there are no rows', () => {
		render(Table, {
			props: {
				columns,
				rows: [],
				rowKey: (row: Row) => row.id,
				emptyTitle: 'Nothing here'
			}
		});
		expect(screen.getByText('Nothing here')).toBeInTheDocument();
	});
});
