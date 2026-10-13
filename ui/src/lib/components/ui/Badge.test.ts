import { render, screen } from '@testing-library/svelte';
import { createRawSnippet, type Snippet } from 'svelte';
import { describe, expect, it } from 'vitest';
import Badge from './Badge.svelte';

function label(value: string): Snippet {
	return createRawSnippet(() => ({ render: () => `<span>${value}</span>` }));
}

describe('Badge', () => {
	it('renders the tone as a class', () => {
		render(Badge, { props: { tone: 'success', children: label('Online') } });
		expect(screen.getByText('Online').closest('.badge')).toHaveClass('badge--success');
	});
});
