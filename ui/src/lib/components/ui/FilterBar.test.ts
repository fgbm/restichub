import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
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
});
