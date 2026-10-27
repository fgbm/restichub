import { render, screen, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { tick } from 'svelte';
import { describe, expect, it, vi } from 'vitest';
import type { Agent } from '#lib/api/index.ts';
import { hostState, iconOf, pillOf } from '#lib/vocabulary.ts';
import ListView from './ListView.svelte';
import type { ListMeta, RowAction } from './types';

interface Item {
	id: string;
	name: string;
	at?: string;
	online?: boolean;
}

const base = {
	label: 'Items',
	itemKey: (item: Item) => item.id,
	icon: () => ({ name: 'host' as const }),
	title: (item: Item) => item.name
};

const action = (id: string): RowAction => ({ id, label: id, icon: 'copy', onSelect: vi.fn() });

function items(count: number): Item[] {
	return Array.from({ length: count }, (_, index) => ({ id: `i${index}`, name: `item-${index}` }));
}

function agent(patch: Partial<Agent>): Agent {
	return {
		id: 'a1',
		hostname: 'srv-db-01',
		status: 'active',
		online: true,
		first_connected_at: '2026-01-01T00:00:00Z',
		certificate_expires_at: '2099-01-01T00:00:00Z',
		version_status: 'current',
		...patch
	} as Agent;
}

describe('ListView', () => {
	it('makes the title the only link of an item', () => {
		const { container } = render(ListView<Item>, {
			props: {
				...base,
				items: items(2),
				href: (item: Item) => `/hosts/${item.id}`,
				meta: () => ['Linux', 'agent 0.4.2'],
				actions: () => [action('Copy')]
			}
		});
		const rows = container.querySelectorAll('li.item');
		expect(rows).toHaveLength(2);
		for (const row of rows) {
			const links = row.querySelectorAll('a');
			expect(links).toHaveLength(1);
			expect(links[0]).toHaveClass('item__title');
		}
		expect(screen.getByRole('link', { name: 'item-0' })).toHaveAttribute('href', '/hosts/i0');
		expect(rows[0]).not.toHaveAttribute('onclick');
	});

	it('opens a side panel from the title of an entity without a record page', async () => {
		const onOpen = vi.fn();
		render(ListView<Item>, { props: { ...base, items: items(1), onOpen } });
		await userEvent.click(screen.getByRole('button', { name: 'item-0' }));
		expect(onOpen).toHaveBeenCalledWith(expect.objectContaining({ id: 'i0' }));
		expect(screen.queryByRole('link')).toBeNull();
	});

	it('shows one or two actions as icon buttons and three or more as one … button', () => {
		const { container } = render(ListView<Item>, {
			props: {
				...base,
				items: [
					{ id: 'two', name: 'two' },
					{ id: 'three', name: 'three' }
				],
				actions: (item: Item) =>
					item.id === 'two'
						? [action('Copy'), action('Delete')]
						: [action('Copy'), action('Delete'), action('Revoke')],
				moreLabel: (item: Item) => `More for ${item.name}`
			}
		});
		const [two, three] = container.querySelectorAll('li.item');
		expect(within(two as HTMLElement).getAllByRole('button')).toHaveLength(2);
		const controls = within(three as HTMLElement).getAllByRole('button');
		expect(controls).toHaveLength(1);
		expect(controls[0]).toHaveAccessibleName('More for three');
	});

	it('shows a pill only for an unusual state', () => {
		const hosts = [
			agent({ id: 'on', hostname: 'on' }),
			agent({ id: 'off', hostname: 'off', online: false })
		];
		const { container } = render(ListView<Agent>, {
			props: {
				label: 'Hosts',
				items: hosts,
				itemKey: (host: Agent) => host.id,
				title: (host: Agent) => host.hostname,
				icon: (host: Agent) => iconOf(hostState(host)),
				pill: (host: Agent) => pillOf(hostState(host))
			}
		});
		const [online, offline] = container.querySelectorAll('li.item');
		expect(online.querySelector('.status')).toBeNull();
		expect(offline.querySelector('.status')).toHaveTextContent('Offline');
		expect(offline.querySelector('.item__lead')).toHaveClass('tone--warning');
		expect(online.querySelector('.item__lead')).not.toHaveClass('tone--warning');
	});

	it('joins the fields in order, leaves out empty ones and marks the fields past the second', () => {
		const meta = (): ListMeta[] => [
			'first',
			null,
			{ text: 'second', mono: true },
			'',
			{ text: 'third', time: '2026-10-16T09:40:02Z' },
			'fourth'
		];
		const { container } = render(ListView<Item>, {
			props: { ...base, items: items(1), meta, aside: () => '4 min 12 s' }
		});
		const fields = [...container.querySelectorAll('.item__field:not(.item__field--aside)')];
		expect(fields.map((field) => field.textContent?.replace(/\s+/g, ' ').trim())).toEqual([
			'first',
			'· second',
			'· third',
			'· fourth'
		]);
		expect(fields.map((field) => field.classList.contains('item__field--extra'))).toEqual([
			false,
			false,
			true,
			true
		]);
		expect(container.querySelector('time')).toHaveAttribute('datetime', '2026-10-16T09:40:02Z');
		expect(container.querySelector('.item__aside')).toHaveTextContent('4 min 12 s');
		expect(container.querySelector('.item__field--aside')).toHaveTextContent('4 min 12 s');
	});

	it('a_snapshot_row_folds_the_tags_that_do_not_fit', async () => {
		const user = userEvent.setup();
		const meta = (): ListMeta[] => [
			{
				tags: [
					{
						label: 'Office files',
						tone: 'running' as const,
						href: '/profiles/p_1',
						title: 'Open profile «Office files»'
					},
					{ label: 'daily' },
					{ label: 'monthly' },
					{ label: 'offsite' }
				]
			}
		];
		const { container } = render(ListView<Item>, { props: { ...base, items: items(1), meta } });

		const line = [...container.querySelectorAll('.item__meta .item__tag')];
		expect(line.map((chip) => chip.textContent)).toEqual(['Office files', 'daily', '+2']);
		expect(container.querySelector('.item__meta a.item__tag')).toHaveAttribute(
			'href',
			'/profiles/p_1'
		);

		screen.getByRole('button', { name: 'All tags (4)' }).focus();
		await user.keyboard('{Enter}');

		const all = [...document.querySelectorAll('.item__tags--all .item__tag')];
		expect(all.map((chip) => chip.textContent)).toEqual([
			'Office files',
			'daily',
			'monthly',
			'offsite'
		]);
		expect(all[0]).toHaveAttribute('href', '/profiles/p_1');
	});

	it('puts a heading before the first item of each group', () => {
		const list: Item[] = [
			{ id: 'a', name: '09:43', at: 'Today' },
			{ id: 'b', name: '02:04', at: 'Today' },
			{ id: 'c', name: '02:03', at: 'Yesterday' }
		];
		const { container } = render(ListView<Item>, {
			props: { ...base, items: list, groupBy: (item: Item) => item.at ?? '' }
		});
		const entries = [...container.querySelectorAll('ul.list > li')].map((li) =>
			li.classList.contains('group')
				? `# ${li.textContent}`
				: li.querySelector('.item__title')?.textContent
		);
		expect(entries).toEqual(['# Today', '09:43', '02:04', '# Yesterday', '02:03']);
	});

	it('tells a first run apart from no matches', async () => {
		const onReset = vi.fn();
		const { rerender } = render(ListView<Item>, {
			props: { ...base, items: [], emptyTitle: 'No hosts yet', emptyIcon: 'host' as const }
		});
		expect(screen.getByText('No hosts yet')).toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Reset filters' })).toBeNull();

		await rerender({ ...base, items: [], filtered: true, onReset, emptyTitle: 'No hosts yet' });
		expect(screen.getByText('Nothing found')).toBeInTheDocument();
		await userEvent.click(screen.getByRole('button', { name: 'Reset filters' }));
		expect(onReset).toHaveBeenCalledOnce();
	});

	it('shows placeholder items only when the load lasts longer than the delay', async () => {
		vi.useFakeTimers();
		try {
			const { container } = render(ListView<Item>, {
				props: { ...base, items: [], loading: true }
			});
			expect(container.querySelector('.item--skeleton')).toBeNull();

			await vi.advanceTimersByTimeAsync(200);
			await tick();

			expect(container.querySelector('ul.list')).toHaveAttribute('aria-busy', 'true');
			expect(container.querySelectorAll('.item--skeleton').length).toBeGreaterThan(0);
		} finally {
			vi.useRealTimers();
		}
	});

	it('shows no placeholder when the load ends at once', async () => {
		vi.useFakeTimers();
		try {
			const { container, rerender } = render(ListView<Item>, {
				props: { ...base, items: [], loading: true }
			});
			await rerender({ ...base, items: items(2), loading: false });

			await vi.advanceTimersByTimeAsync(300);

			expect(container.querySelector('.item--skeleton')).toBeNull();
			expect(screen.getByText('item-0')).toBeInTheDocument();
		} finally {
			vi.useRealTimers();
		}
	});

	it('pages a client list with the standard footer', async () => {
		render(ListView<Item>, { props: { ...base, items: items(30), pageSize: 25 } });
		expect(screen.queryAllByRole('link')).toHaveLength(0);
		expect(screen.getByText('item-24')).toBeInTheDocument();
		expect(screen.queryByText('item-25')).toBeNull();
		await userEvent.click(screen.getByRole('button', { name: 'Page 2' }));
		expect(screen.getByText('item-29')).toBeInTheDocument();
	});
});
