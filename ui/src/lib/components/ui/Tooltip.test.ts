import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { createRawSnippet, type Snippet } from 'svelte';
import { describe, expect, it } from 'vitest';
import Tooltip from './Tooltip.svelte';

function trigger(value: string): Snippet {
	return createRawSnippet(() => ({ render: () => `<span>${value}</span>` }));
}

describe('Tooltip', () => {
	it('shows the content on hover', async () => {
		const user = userEvent.setup();
		render(Tooltip, { props: { content: 'Copy the value', children: trigger('Help') } });

		await user.hover(screen.getByRole('button', { name: 'Help' }));
		expect(await screen.findByText('Copy the value')).toBeInTheDocument();
	});
});
