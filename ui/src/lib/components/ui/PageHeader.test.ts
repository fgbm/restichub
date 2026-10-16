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

	it('puts the context before the actions on the title row, with a label without a colon', () => {
		const { container } = render(PageHeader, {
			props: {
				title: 'Restore',
				description: 'Pick a snapshot',
				context: createRawSnippet(() => ({
					render: () => '<label>Repository <select><option>main</option></select></label>'
				})),
				children: action()
			}
		});
		const side = container.querySelector('.page-header__side');
		expect(side?.previousElementSibling).toHaveClass('page-header__title-row');
		expect(side?.firstElementChild).toHaveClass('page-header__context');
		expect(side?.lastElementChild).toContainElement(
			screen.getByRole('button', { name: 'New host' })
		);
		expect(screen.getByRole('combobox', { name: 'Repository' })).toBeInTheDocument();
	});
});
