import { render, screen } from '@testing-library/svelte';
import { createRawSnippet, type Snippet } from 'svelte';
import { describe, expect, it } from 'vitest';
import StatusPill from './StatusPill.svelte';

function label(value: string): Snippet {
	return createRawSnippet(() => ({ render: () => `<span>${value}</span>` }));
}

describe('StatusPill', () => {
	it('renders the status tone as a class', () => {
		render(StatusPill, { props: { tone: 'running', children: label('Running') } });
		expect(screen.getByText('Running').closest('.status')).toHaveClass('status--running');
	});
});
