import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { createRawSnippet } from 'svelte';
import { describe, expect, it } from 'vitest';
import Popover from './Popover.svelte';

const text = (value: string) => createRawSnippet(() => ({ render: () => `<span>${value}</span>` }));

describe('Popover', () => {
	it('opens on click and closes on Escape', async () => {
		const user = userEvent.setup();
		render(Popover, {
			props: { label: 'Filters', trigger: text('Filters'), children: text('Panel body') }
		});

		expect(screen.queryByText('Panel body')).not.toBeInTheDocument();
		await user.click(screen.getByRole('button', { name: 'Filters' }));
		expect(await screen.findByText('Panel body')).toBeInTheDocument();
		await user.keyboard('{Escape}');
		expect(screen.queryByText('Panel body')).not.toBeInTheDocument();
	});
});
