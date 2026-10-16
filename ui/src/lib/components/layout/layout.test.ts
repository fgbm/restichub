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

	it('makes the sections and the footer one form', async () => {
		const onsubmit = vi.fn();
		const { container } = render(RecordPage, {
			props: {
				title: 'New profile',
				onsubmit,
				footer: html('<button type="submit">Save</button>'),
				children: html('<section>Basics</section>')
			}
		});
		const form = container.querySelector('form');
		expect(form).not.toBeNull();
		await fireEvent.submit(form as HTMLFormElement);
		expect(onsubmit).toHaveBeenCalledOnce();
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
