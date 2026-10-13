import { render, screen } from '@testing-library/svelte';
import { createRawSnippet, type Snippet } from 'svelte';
import { describe, expect, it } from 'vitest';
import Card from './Card.svelte';

function text(value: string): Snippet {
	return createRawSnippet(() => ({ render: () => `<p>${value}</p>` }));
}

describe('Card', () => {
	it('renders the content and is padded by default', () => {
		const { container } = render(Card, { props: { children: text('Body') } });
		expect(screen.getByText('Body')).toBeInTheDocument();
		expect(container.querySelector('.card')).toHaveClass('card--padded');
	});
});
