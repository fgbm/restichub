import { render, screen } from '@testing-library/svelte';
import { createRawSnippet, type Snippet } from 'svelte';
import { describe, expect, it } from 'vitest';
import Alert from './Alert.svelte';

function text(value: string): Snippet {
	return createRawSnippet(() => ({ render: () => `<span>${value}</span>` }));
}

describe('Alert', () => {
	it('uses the alert role for a danger callout', () => {
		render(Alert, { props: { tone: 'danger', title: 'The save failed', children: text('Retry') } });
		expect(screen.getByRole('alert')).toHaveTextContent('The save failed');
	});

	it('uses the status role for an info callout', () => {
		render(Alert, { props: { tone: 'info', title: 'Note' } });
		expect(screen.getByRole('status')).toHaveTextContent('Note');
	});
});
