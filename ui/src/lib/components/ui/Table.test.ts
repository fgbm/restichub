import { render, screen } from '@testing-library/svelte';
import type { Component } from 'svelte';
import { createRawSnippet, type Snippet } from 'svelte';
import { describe, expect, it } from 'vitest';
import TableComponent from './Table.svelte';
import type { TableColumn } from './types';

interface Row {
	id: string;
	name: string;
}

interface TableProps {
	columns: TableColumn<Row>[];
	rows: Row[];
	rowKey: (row: Row) => string;
	loading?: boolean;
	flush?: boolean;
	rowActions?: Snippet<[Row]>;
}

// The generic component is used with one row type in these tests.
const Table = TableComponent as unknown as Component<TableProps>;

const columns: TableColumn<Row>[] = [{ key: 'name', header: 'Name' }];
const rows: Row[] = [
	{ id: 'r1', name: 'host-01' },
	{ id: 'r2', name: 'host-02' }
];

describe('Table', () => {
	it('renders the rows and the header', () => {
		render(Table, { props: { columns, rows, rowKey: (row) => row.id } });
		expect(screen.getByRole('columnheader', { name: 'Name' })).toBeInTheDocument();
		expect(screen.getByText('host-01')).toBeInTheDocument();
	});

	it('shows the empty state without rows', () => {
		render(Table, { props: { columns, rows: [], rowKey: (row) => row.id } });
		expect(screen.getByText('No records')).toBeInTheDocument();
	});

	it('shows skeleton rows while loading', () => {
		const { container } = render(Table, {
			props: { columns, rows: [], rowKey: (row) => row.id, loading: true }
		});
		expect(container.querySelectorAll('tbody tr')).toHaveLength(10);
	});

	it('keeps a nowrap column on one line', () => {
		const { container } = render(Table, {
			props: {
				columns: [{ key: 'name', header: 'Name', nowrap: true }],
				rows,
				rowKey: (row) => row.id
			}
		});
		expect(screen.getByRole('columnheader', { name: 'Name' })).toHaveClass('table__cell--nowrap');
		const cells = container.querySelectorAll('tbody td');
		expect(cells).toHaveLength(2);
		for (const cell of cells) expect(cell).toHaveClass('table__cell--nowrap');
	});

	it('wraps a column that has no nowrap flag', () => {
		const { container } = render(Table, { props: { columns, rows, rowKey: (row) => row.id } });
		expect(container.querySelector('.table__cell--nowrap')).toBeNull();
	});

	it('renders the row actions', () => {
		const rowActions = createRawSnippet<[Row]>((row) => ({
			render: () => `<button type="button">Open ${row().id}</button>`
		}));
		render(Table, { props: { columns, rows, rowKey: (row) => row.id, rowActions } });
		expect(screen.getByRole('button', { name: 'Open r1' })).toBeInTheDocument();
	});

	it('has no inner height limit, so the table grows with the page', () => {
		const { container } = render(Table, { props: { columns, rows, rowKey: (row) => row.id } });
		expect(container.querySelector('.table-scroll')).not.toBeNull();
		expect(container.querySelector('.table-wrap--flush')).toBeNull();
	});

	it('marks a table inside a panel as flush', () => {
		const { container } = render(Table, {
			props: { columns, rows, rowKey: (row) => row.id, flush: true }
		});
		expect(container.querySelector('.table-wrap')).toHaveClass('table-wrap--flush');
	});

	it('marks a column that hides on a narrow screen', () => {
		render(Table, {
			props: {
				columns: [{ key: 'name', header: 'Name', hideNarrow: true }],
				rows,
				rowKey: (row: Row) => row.id
			}
		});
		expect(screen.getByRole('columnheader', { name: 'Name' })).toHaveClass(
			'table__cell--narrow-hidden'
		);
	});
});
