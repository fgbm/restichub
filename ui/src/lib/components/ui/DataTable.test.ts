import { fireEvent, render, screen, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import type { Component } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import DataTableComponent from './DataTable.svelte';
import type { RowAction, TableColumn, TableSort } from './types';

interface Row {
	id: string;
	name: string;
	seen?: string;
}

interface DataTableProps {
	columns: TableColumn<Row>[];
	rows: Row[];
	rowKey: (row: Row) => string;
	rowHref?: (row: Row) => string;
	onRowOpen?: (row: Row) => void;
	actions?: (row: Row) => RowAction[];
	mode?: 'client' | 'server';
	page?: number;
	pageSize?: number;
	total?: number;
	sort?: TableSort;
	onPageChange?: (page: number) => void;
	onSortChange?: (sort: TableSort) => void;
}

// The generic component is used with one row type in these tests.
const DataTable = DataTableComponent as unknown as Component<DataTableProps>;

const columns: TableColumn<Row>[] = [
	{ key: 'name', header: 'Name' },
	{ key: 'id', header: 'ID' }
];
const rows: Row[] = [
	{ id: 'r1', name: 'host-01' },
	{ id: 'r2', name: 'host-02' }
];
const rowKey = (row: Row) => row.id;

function action(id: string, onSelect: () => void = () => {}): RowAction {
	return { id, label: `Action ${id}`, icon: 'x', onSelect };
}

afterEach(() => {
	vi.restoreAllMocks();
	window.getSelection()?.removeAllRanges();
});

describe('DataTable', () => {
	it('puts a real link to the record in the first cell', () => {
		render(DataTable, { props: { columns, rows, rowKey, rowHref: (row) => `/hosts/${row.id}` } });
		const link = screen.getByRole('link', { name: 'host-01' });
		expect(link).toHaveAttribute('href', '/hosts/r1');
		expect(link).toHaveClass('row-link');
	});

	it('opens the record on a click anywhere in the row', async () => {
		const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
		render(DataTable, { props: { columns, rows, rowKey, rowHref: (row) => `/hosts/${row.id}` } });
		await fireEvent.click(screen.getByText('r2'));
		expect(click).toHaveBeenCalledTimes(1);
		expect(click.mock.contexts[0]).toHaveAttribute('href', '/hosts/r2');
	});

	it('opens the record in a new tab on a Ctrl click and a middle click', async () => {
		const open = vi.spyOn(window, 'open').mockImplementation(() => null);
		render(DataTable, { props: { columns, rows, rowKey, rowHref: (row) => `/hosts/${row.id}` } });
		await fireEvent.click(screen.getByText('r1'), { ctrlKey: true });
		await fireEvent(
			screen.getByText('r2'),
			new MouseEvent('auxclick', { bubbles: true, button: 1 })
		);
		expect(open).toHaveBeenNthCalledWith(1, '/hosts/r1', '_blank', 'noopener');
		expect(open).toHaveBeenNthCalledWith(2, '/hosts/r2', '_blank', 'noopener');
	});

	it('does not open the record when the click ends a text selection', async () => {
		const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
		render(DataTable, { props: { columns, rows, rowKey, rowHref: (row) => `/hosts/${row.id}` } });
		const cell = screen.getByText('r1');
		const range = document.createRange();
		range.selectNodeContents(cell);
		window.getSelection()?.addRange(range);
		await fireEvent.click(cell);
		expect(click).not.toHaveBeenCalled();
	});

	it('does not open the record from a row control', async () => {
		const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
		const onSelect = vi.fn();
		render(DataTable, {
			props: {
				columns,
				rows,
				rowKey,
				rowHref: (row) => `/hosts/${row.id}`,
				actions: () => [action('a', onSelect)]
			}
		});
		await userEvent.click(screen.getAllByRole('button', { name: 'Action a' })[0]);
		expect(onSelect).toHaveBeenCalledTimes(1);
		expect(click).not.toHaveBeenCalled();
	});

	it('opens a side panel for a table without a record page', async () => {
		const onRowOpen = vi.fn();
		render(DataTable, { props: { columns, rows, rowKey, onRowOpen } });
		await userEvent.click(screen.getByText('r2'));
		expect(onRowOpen).toHaveBeenCalledWith(rows[1]);
		await userEvent.click(screen.getByRole('button', { name: 'host-01' }));
		expect(onRowOpen).toHaveBeenLastCalledWith(rows[0]);
	});

	it('shows one or two row actions as icon buttons with a label and a tooltip', () => {
		render(DataTable, {
			props: { columns, rows: [rows[0]], rowKey, actions: () => [action('a'), action('b')] }
		});
		const row = screen.getAllByRole('row')[1];
		const buttons = within(row).getAllByRole('button');
		expect(buttons).toHaveLength(2);
		expect(buttons[0]).toHaveAccessibleName('Action a');
		expect(buttons[0]).toHaveAttribute('title', 'Action a');
		expect(buttons[0].textContent?.trim()).toBe('');
	});

	it('puts three or more row actions in one menu button', async () => {
		render(DataTable, {
			props: {
				columns,
				rows: [rows[0]],
				rowKey,
				actions: () => [action('a'), action('b'), action('c')]
			}
		});
		const row = screen.getAllByRole('row')[1];
		const buttons = within(row).getAllByRole('button');
		expect(buttons).toHaveLength(1);
		await userEvent.setup().click(buttons[0]);
		expect(await screen.findByText('Action c')).toHaveAttribute('role', 'menuitem');
	});

	it('never renders more than two row controls', () => {
		for (let count = 0; count <= 6; count += 1) {
			const list = Array.from({ length: count }, (_, index) => action(String(index)));
			const { container, unmount } = render(DataTable, {
				props: { columns, rows: [rows[0]], rowKey, actions: () => list }
			});
			const controls = container.querySelectorAll('td.table__cell--actions button');
			expect(controls.length).toBeLessThanOrEqual(2);
			expect(controls.length).toBe(count > 2 ? 1 : count);
			unmount();
		}
	});

	it('leaves out a hidden action and counts only the visible ones', () => {
		render(DataTable, {
			props: {
				columns,
				rows: [rows[0]],
				rowKey,
				actions: () => [action('a'), action('b'), { ...action('c'), hidden: true }]
			}
		});
		const row = screen.getAllByRole('row')[1];
		expect(within(row).getAllByRole('button')).toHaveLength(2);
		expect(within(row).queryByRole('button', { name: 'Action c' })).toBeNull();
	});

	it('cuts a long value and shows the full value in a tooltip', () => {
		const long = 'a-very-long-host-name-that-does-not-fit';
		render(DataTable, {
			props: {
				columns: [{ key: 'name', header: 'Name', truncate: true }],
				rows: [{ id: 'r1', name: long }],
				rowKey
			}
		});
		const cell = screen.getByText(long);
		expect(cell).toHaveClass('table__truncate');
		expect(cell).toHaveAttribute('title', long);
	});

	it('keeps a date on one line in a time element', () => {
		const { container } = render(DataTable, {
			props: {
				columns: [
					{ key: 'name', header: 'Name' },
					{ key: 'seen', header: 'Seen', date: 'relative' }
				],
				rows: [{ id: 'r1', name: 'host-01', seen: '2026-01-02T03:04:05Z' }],
				rowKey
			}
		});
		const time = container.querySelector('time');
		expect(time).toHaveAttribute('datetime', '2026-01-02T03:04:05Z');
		expect(time?.closest('td')).toHaveClass('table__cell--nowrap');
	});

	it('pages on the client', async () => {
		const many = Array.from({ length: 12 }, (_, index) => ({ id: `r${index}`, name: `h${index}` }));
		render(DataTable, { props: { columns, rows: many, rowKey, pageSize: 5 } });
		expect(screen.getAllByRole('row')).toHaveLength(6);
		await userEvent.click(screen.getByRole('button', { name: 'Next' }));
		expect(screen.getByText('h5')).toBeInTheDocument();
		expect(screen.getByText('Showing 6 to 10 of 12')).toBeInTheDocument();
	});

	it('in server mode shows the rows as given and asks for another page and order', async () => {
		const onPageChange = vi.fn();
		const onSortChange = vi.fn();
		render(DataTable, {
			props: {
				columns,
				rows,
				rowKey,
				mode: 'server',
				page: 2,
				pageSize: 2,
				total: 7,
				onPageChange,
				onSortChange
			}
		});
		expect(screen.getByText('Showing 3 to 4 of 7')).toBeInTheDocument();
		await userEvent.click(screen.getByRole('button', { name: 'Next' }));
		expect(onPageChange).toHaveBeenCalledWith(3);
		await userEvent.click(screen.getByRole('button', { name: /Name/ }));
		expect(onSortChange).toHaveBeenCalledWith({ key: 'name', direction: 'asc' });
		expect(onPageChange).toHaveBeenLastCalledWith(1);
		expect(screen.getAllByRole('row')[1]).toHaveTextContent('host-01');
	});

	it('sorts on the client when a person sorts a column', async () => {
		render(DataTable, { props: { columns, rows, rowKey } });
		await userEvent.click(screen.getByRole('button', { name: /Name/ }));
		await userEvent.click(screen.getByRole('button', { name: /Name/ }));
		expect(screen.getAllByRole('row')[1]).toHaveTextContent('host-02');
	});
});
