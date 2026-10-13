import { render, screen } from '@testing-library/svelte';
import { createRawSnippet, type Snippet } from 'svelte';
import { describe, expect, it } from 'vitest';
import Panel from './Panel.svelte';

function text(value: string): Snippet {
	return createRawSnippet(() => ({ render: () => `<p>${value}</p>` }));
}

describe('Panel', () => {
	it('renders the title and the body', () => {
		render(Panel, { props: { title: 'Details', children: text('Content') } });
		expect(screen.getByRole('heading', { name: 'Details' })).toBeInTheDocument();
		expect(screen.getByText('Content')).toBeInTheDocument();
	});
});
