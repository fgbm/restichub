import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { createRawSnippet, type Snippet } from 'svelte';
import { describe, expect, it, vi } from 'vitest';
import Toast from './Toast.svelte';

function text(value: string): Snippet {
	return createRawSnippet(() => ({ render: () => `<span>${value}</span>` }));
}

describe('Toast', () => {
	it('shows the message in a status region and dismisses', async () => {
		const user = userEvent.setup();
		const onDismiss = vi.fn();
		render(Toast, {
			props: { tone: 'success', title: 'Saved', onDismiss, children: text('The tenant is ready.') }
		});

		expect(screen.getByRole('status')).toHaveTextContent('Saved');
		await user.click(screen.getByRole('button', { name: 'Dismiss' }));
		expect(onDismiss).toHaveBeenCalledOnce();
	});
});
