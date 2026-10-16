import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { createRawSnippet } from 'svelte';
import { describe, expect, it, vi } from 'vitest';
import FilterBar from './FilterBar.svelte';
import type { FilterFacet } from './types';

const facets: FilterFacet[] = [
	{
		key: 'state',
		label: 'State',
		multiple: false,
		options: [
			{ value: 'online', label: 'Online', count: 3 },
			{ value: 'offline', label: 'Offline', count: 1 }
		]
	},
	{
		key: 'os',
		label: 'OS',
		options: [
			{ value: 'linux', count: 2 },
			{ value: 'windows', count: 2 }
		]
	}
];

const search = { key: 'q', label: 'Name' };

describe('FilterBar', () => {
	it('applies the search text at once', async () => {
		const onchange = vi.fn();
		render(FilterBar, { props: { label: 'Host filters', value: { q: '' }, search, onchange } });
		await userEvent.type(screen.getByRole('searchbox', { name: 'Name' }), 'w');
		expect(onchange).toHaveBeenLastCalledWith({ q: 'w' });
	});

	it('shows the facet values with their counts and adds a value', async () => {
		const user = userEvent.setup();
		const onchange = vi.fn();
		render(FilterBar, {
			props: { label: 'Host filters', value: { os: 'linux' }, facets, onchange }
		});
		await user.click(screen.getByRole('button', { name: /^OS/ }));
		// The floating panel is invisible until it is placed, which jsdom never does.
		const windows = await screen.findByLabelText('windows');
		expect(windows).not.toBeChecked();
		expect(screen.getByLabelText('linux')).toBeChecked();
		await user.click(windows);
		expect(onchange).toHaveBeenLastCalledWith({ os: 'linux,windows' });
	});

	it('chooses one value of a single-value facet', async () => {
		const user = userEvent.setup();
		const onchange = vi.fn();
		render(FilterBar, { props: { label: 'Host filters', value: {}, facets, onchange } });
		await user.click(screen.getByRole('button', { name: /State/ }));
		const online = (await screen.findByText('Online')).closest('button') as HTMLElement;
		expect(online).toHaveTextContent('3');
		await user.click(online);
		expect(onchange).toHaveBeenLastCalledWith({ state: 'online' });
	});

	it('shows a chip for each applied filter and removes one with its button', async () => {
		const onchange = vi.fn();
		render(FilterBar, {
			props: {
				label: 'Host filters',
				value: { q: 'web', state: 'offline', os: 'linux,windows', period: '7d' },
				search,
				facets,
				period: true,
				onchange
			}
		});
		const chips = screen.getByRole('list', { name: 'Applied filters' });
		expect(chips).toHaveTextContent('Name: web');
		expect(chips).toHaveTextContent('State: Offline');
		expect(chips).toHaveTextContent('OS: windows');
		expect(chips).toHaveTextContent('Period: Last 7 days');
		await userEvent.click(screen.getByRole('button', { name: 'Remove the filter OS: linux' }));
		expect(onchange).toHaveBeenLastCalledWith({ os: 'windows' });
		await userEvent.click(
			screen.getByRole('button', { name: 'Remove the filter Period: Last 7 days' })
		);
		expect(onchange).toHaveBeenLastCalledWith({ period: '', from: '', to: '' });
	});

	it('resets all its filters and shows Reset only when a filter applies', async () => {
		const onchange = vi.fn();
		const { rerender } = render(FilterBar, {
			props: { label: 'Host filters', value: { q: '', os: '' }, search, facets, onchange }
		});
		expect(screen.queryByRole('button', { name: 'Reset' })).toBeNull();
		await rerender({ value: { q: 'x', os: '' } });
		await userEvent.click(screen.getByRole('button', { name: 'Reset' }));
		expect(onchange).toHaveBeenLastCalledWith({ q: '', state: '', os: '' });
	});

	it('offers the period presets and a custom range', async () => {
		const user = userEvent.setup();
		const onchange = vi.fn();
		render(FilterBar, {
			props: { label: 'Job filters', value: { period: 'custom' }, period: true, onchange }
		});
		await user.click(screen.getByRole('button', { name: /Period/ }));
		expect(await screen.findByLabelText('From')).toHaveAttribute('type', 'date');
		await user.click(screen.getByText('Last 24 hours'));
		expect(onchange).toHaveBeenLastCalledWith({ period: '24h', from: '', to: '' });
	});

	it('takes a free text in a facet without options', async () => {
		const user = userEvent.setup();
		const onchange = vi.fn();
		render(FilterBar, {
			props: {
				label: 'Audit filters',
				value: { object_id: 'r_1' },
				facets: [{ key: 'object_id', label: 'Object ID' }],
				onchange
			}
		});
		expect(screen.getByRole('list', { name: 'Applied filters' })).toHaveTextContent(
			'Object ID: r_1'
		);
		await user.click(screen.getByRole('button', { name: /^Object ID/ }));
		const field = await waitFor(() => document.getElementById('filter-object_id') as HTMLElement);
		// The focus scope of the popover settles late in jsdom, so the test types without the pointer.
		await fireEvent.input(field, { target: { value: 'r_12' } });
		expect(onchange).toHaveBeenLastCalledWith({ object_id: 'r_12' });
	});

	it('shows the hint of a facet in its popover and puts the end content in the row', async () => {
		const user = userEvent.setup();
		const { container } = render(FilterBar, {
			props: {
				label: 'Snapshot filters',
				value: {},
				facets: [{ ...facets[1], hint: 'Any chosen value matches.' }],
				end: createRawSnippet(() => ({ render: () => '<button type="button">Refresh</button>' })),
				onchange: vi.fn()
			}
		});
		expect(screen.queryByText('Any chosen value matches.')).toBeNull();
		expect(container.querySelector('.filter-bar__row .filter-bar__end')).toContainElement(
			screen.getByRole('button', { name: 'Refresh' })
		);
		await user.click(screen.getByRole('button', { name: /^OS/ }));
		expect(await screen.findByText('Any chosen value matches.')).toBeInTheDocument();
	});
});

describe('FilterBar in a list view', () => {
	const context = new Map([['rh-list-view', true]]);
	const states = {
		default: 'all',
		options: [
			{ value: 'all', label: 'All', count: 124 },
			{ value: 'failed', label: 'Failed', count: 3, danger: true },
			{ value: 'running', label: 'Running', count: 0, danger: false }
		]
	};
	const sort = {
		default: 'name',
		options: [
			{ value: 'name', label: 'Name' },
			{ value: 'contact', label: 'Last contact', order: 'desc' as const }
		]
	};

	it('shows the state tabs with counts and writes the state key', async () => {
		const onchange = vi.fn();
		render(FilterBar, {
			props: { label: 'Job filters', value: {}, states, onchange },
			context
		});
		const all = screen.getByRole('button', { name: 'All 124' });
		expect(all).toHaveAttribute('aria-pressed', 'true');
		const failed = screen.getByRole('button', { name: 'Failed 3' });
		expect(failed.querySelector('.count')).toHaveClass('count--danger');
		await userEvent.click(failed);
		expect(onchange).toHaveBeenLastCalledWith({ state: 'failed' });
		await userEvent.click(all);
		expect(onchange).toHaveBeenLastCalledWith({ state: '' });
	});

	it('makes the count red only for the failure tab with a count above zero', () => {
		render(FilterBar, {
			props: {
				label: 'Job filters',
				value: {},
				states: {
					...states,
					options: [{ value: 'failed', label: 'Failed', count: 0, danger: true }]
				},
				onchange: vi.fn()
			},
			context
		});
		expect(
			screen.getByRole('button', { name: 'Failed 0' }).querySelector('.count')
		).not.toHaveClass('count--danger');
	});

	it('shows the total in place of the tabs of an entity without states', () => {
		render(FilterBar, {
			props: { label: 'Profile filters', value: {}, total: 7, onchange: vi.fn() },
			context
		});
		expect(screen.getByText('Total')).toHaveTextContent('Total 7');
	});

	it('sets the sort field and the order in the Sort menu', async () => {
		const user = userEvent.setup();
		const onchange = vi.fn();
		render(FilterBar, {
			props: { label: 'Host filters', value: {}, sort, onchange },
			context
		});
		await user.click(screen.getByRole('button', { name: /Name/ }));
		await user.click(await screen.findByText('Last contact'));
		expect(onchange).toHaveBeenLastCalledWith({ sort: 'contact', order: '' });
	});

	it('resets the filters and the state tab but keeps the sort', async () => {
		const onchange = vi.fn();
		render(FilterBar, {
			props: {
				label: 'Host filters',
				value: { q: 'web', state: 'failed', sort: 'contact' },
				search,
				states,
				sort,
				onchange
			},
			context
		});
		const user = userEvent.setup({ pointerEventsCheck: 0 });
		await user.click(screen.getByRole('button', { name: 'Reset' }));
		expect(onchange).toHaveBeenLastCalledWith({ q: '', state: '' });
	});
});
