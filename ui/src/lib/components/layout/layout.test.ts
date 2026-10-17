import { fireEvent, render, screen } from '@testing-library/svelte';
import { createRawSnippet, type Snippet } from 'svelte';
import { describe, expect, it, vi } from 'vitest';
import { getPageMeta } from '#lib/stores/page-meta.svelte.ts';
import Page from './Page.svelte';
import RecordPage from './RecordPage.svelte';
import SummaryPanel from './SummaryPanel.svelte';

function html(markup: string): Snippet {
	return createRawSnippet(() => ({ render: () => markup }));
}

describe('Page', () => {
	it('declares its breadcrumb levels while it is shown', () => {
		const { unmount } = render(Page, {
			props: {
				title: 'db-01',
				crumbs: [{ label: 'db-01' }],
				listQuery: 'state=online',
				children: html('<p>Body</p>')
			}
		});
		expect(getPageMeta()).toEqual({ crumbs: [{ label: 'db-01' }], listQuery: 'state=online' });
		unmount();
		expect(getPageMeta()).toBeNull();
	});

	it('renders the header with the context, the status and the actions', () => {
		render(Page, {
			props: {
				title: 'Snapshots',
				description: 'Pick a snapshot',
				context: html('<label>Repository <select><option>main</option></select></label>'),
				status: html('<span>Online</span>'),
				actions: html('<button type="button">Refresh</button>'),
				children: html('<p>Body</p>')
			}
		});
		expect(screen.getByRole('heading', { level: 1, name: 'Snapshots' })).toBeInTheDocument();
		expect(screen.getByRole('combobox', { name: 'Repository' })).toBeInTheDocument();
		expect(screen.getByText('Online')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Refresh' })).toBeInTheDocument();
		expect(screen.getByText('Body')).toBeInTheDocument();
	});

	it('shows the load error instead of the body, without a link back', () => {
		render(Page, {
			props: { title: '', error: 'The host does not exist.', children: html('<p>Body</p>') }
		});
		expect(screen.getByText('The host does not exist.')).toBeInTheDocument();
		expect(screen.queryByText('Body')).toBeNull();
		expect(screen.queryByRole('link')).toBeNull();
	});
});

describe('RecordPage', () => {
	it('puts the sections on the left and the side panel in an aside', () => {
		const { container } = render(RecordPage, {
			props: {
				title: 'db-01',
				side: html('<p>Summary</p>'),
				children: html('<section>General</section>')
			}
		});
		expect(container.querySelector('.record__main')).toHaveTextContent('General');
		expect(screen.getByRole('complementary')).toHaveTextContent('Summary');
		expect(getPageMeta()?.crumbs).toEqual([{ label: 'db-01' }]);
	});

	it('makes the sections and the save bar one form', async () => {
		const onsubmit = vi.fn();
		const oncancel = vi.fn();
		const { container } = render(RecordPage, {
			props: {
				title: 'db-profile',
				onsubmit,
				save: { dirty: true, oncancel },
				children: html('<section>Basics</section>')
			}
		});
		expect(container.querySelector('.record__bar')).toHaveTextContent('The changes are not saved');
		await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
		expect(oncancel).toHaveBeenCalledOnce();
		await fireEvent.submit(container.querySelector('form') as HTMLFormElement);
		expect(onsubmit).toHaveBeenCalledOnce();
	});

	it('disables Save without changes and says why', () => {
		render(RecordPage, {
			props: {
				title: 'db-profile',
				onsubmit: vi.fn(),
				save: { dirty: false, note: 'Used by 2 plans', oncancel: vi.fn() },
				children: html('<section>Basics</section>')
			}
		});
		const save = screen.getByRole('button', { name: 'Save' });
		expect(save).toBeDisabled();
		expect(save).toHaveAccessibleDescription(/No changes.*Used by 2 plans/);
		expect(screen.queryByRole('button', { name: 'Cancel' })).toBeNull();
	});

	it('keeps Save enabled for a new record and shows the reason of a read-only page', () => {
		const { unmount } = render(RecordPage, {
			props: {
				title: 'New profile',
				onsubmit: vi.fn(),
				save: { dirty: false, create: true, oncancel: vi.fn() },
				children: html('<section>Basics</section>')
			}
		});
		expect(screen.getByRole('button', { name: 'Save' })).toBeEnabled();
		unmount();

		render(RecordPage, {
			props: {
				title: 'db-profile',
				save: { dirty: false, readonly: 'Your role cannot change a profile.', oncancel: vi.fn() },
				children: html('<section>Basics</section>')
			}
		});
		expect(screen.getByText('Your role cannot change a profile.')).toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Save' })).toBeNull();
	});
});

describe('SummaryPanel', () => {
	it('shows each label above its value', () => {
		const { container } = render(SummaryPanel, {
			props: { title: 'About', items: [{ key: 'os', label: 'OS', value: 'Linux' }] }
		});
		expect(container.querySelector('.key-value--stacked')).not.toBeNull();
		expect(screen.getByText('OS').tagName).toBe('DT');
		expect(screen.getByText('Linux').tagName).toBe('DD');
	});

	it('does not appear without content', () => {
		const { container } = render(SummaryPanel, { props: { title: 'About', items: [] } });
		expect(container.querySelector('.panel')).toBeNull();
	});
});
