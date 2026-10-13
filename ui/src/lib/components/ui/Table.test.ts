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
		expect(container.querySelectorAll('tbody tr')).toHaveLength(3);
	});

	it('renders the row actions', () => {
		const rowActions = createRawSnippet<[Row]>((row) => ({
			render: () => `<button type="button">Open ${row().id}</button>`
		}));
		render(Table, { props: { columns, rows, rowKey: (row) => row.id, rowActions } });
		expect(screen.getByRole('button', { name: 'Open r1' })).toBeInTheDocument();
	});
});
