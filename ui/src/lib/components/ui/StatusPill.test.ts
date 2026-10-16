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

	it('gives the full meaning as title and accessible name', () => {
		render(StatusPill, {
			props: { tone: 'warning', title: 'Succeeded with warnings', children: label('Warnings') }
		});
		const pill = screen.getByText('Warnings').closest('.status');
		expect(pill).toHaveAttribute('title', 'Succeeded with warnings');
		expect(pill).toHaveAttribute('aria-label', 'Succeeded with warnings');
	});
});
