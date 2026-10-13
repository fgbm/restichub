import { render, screen } from '@testing-library/svelte';
import { createRawSnippet, type Snippet } from 'svelte';
import { describe, expect, it } from 'vitest';
import EmptyState from './EmptyState.svelte';

function action(): Snippet {
	return createRawSnippet(() => ({ render: () => `<button type="button">Add host</button>` }));
}

describe('EmptyState', () => {
	it('renders the title, the description and the action', () => {
		render(EmptyState, {
			props: { title: 'No hosts', description: 'Add the first host.', children: action() }
		});
		expect(screen.getByText('No hosts')).toBeInTheDocument();
		expect(screen.getByText('Add the first host.')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Add host' })).toBeInTheDocument();
	});
});
