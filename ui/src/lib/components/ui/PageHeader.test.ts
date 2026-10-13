import { render, screen } from '@testing-library/svelte';
import { createRawSnippet, type Snippet } from 'svelte';
import { describe, expect, it } from 'vitest';
import PageHeader from './PageHeader.svelte';

function action(): Snippet {
	return createRawSnippet(() => ({ render: () => `<button type="button">New host</button>` }));
}

describe('PageHeader', () => {
	it('renders the title, the description and the actions', () => {
		render(PageHeader, {
			props: { title: 'Hosts', description: 'All hosts', children: action() }
		});
		expect(screen.getByRole('heading', { level: 1, name: 'Hosts' })).toBeInTheDocument();
		expect(screen.getByText('All hosts')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'New host' })).toBeInTheDocument();
	});
});
