import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { createRawSnippet, type Snippet } from 'svelte';
import { describe, expect, it } from 'vitest';
import Tabs from './Tabs.svelte';

const panel = createRawSnippet<[string]>((value) => ({
	render: () => `<p>panel ${value()}</p>`
})) as Snippet<[string]>;

describe('Tabs', () => {
	it('shows the panel of the active tab and switches on a click', async () => {
		const user = userEvent.setup();
		render(Tabs, {
			props: {
				value: 'a',
				label: 'Views',
				items: [
					{ value: 'a', label: 'Alpha', count: 2 },
					{ value: 'b', label: 'Beta' }
				],
				children: panel
			}
		});

		expect(screen.getByRole('tab', { name: /Alpha/ })).toHaveAttribute('data-state', 'active');
		expect(screen.getByText('panel a')).toBeInTheDocument();

		await user.click(screen.getByRole('tab', { name: 'Beta' }));
		expect(screen.getByText('panel b')).toBeInTheDocument();
	});
});
