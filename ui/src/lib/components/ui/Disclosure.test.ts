import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { createRawSnippet } from 'svelte';
import { describe, expect, it } from 'vitest';
import Disclosure from './Disclosure.svelte';

const body = createRawSnippet(() => ({ render: () => '<p>Hidden field</p>' }));

describe('Disclosure', () => {
	it('is closed at first and opens with a click', async () => {
		const user = userEvent.setup();
		render(Disclosure, { props: { label: 'More', children: body } });
		const toggle = screen.getByRole('button', { name: 'More' });
		expect(toggle).toHaveAttribute('aria-expanded', 'false');
		expect(screen.queryByText('Hidden field')).not.toBeInTheDocument();

		await user.click(toggle);
		expect(toggle).toHaveAttribute('aria-expanded', 'true');
		expect(screen.getByText('Hidden field')).toBeInTheDocument();
		expect(toggle).toHaveAttribute(
			'aria-controls',
			screen.getByText('Hidden field').parentElement!.id
		);
	});

	it('toggles with the keyboard', async () => {
		const user = userEvent.setup();
		render(Disclosure, { props: { label: 'More', children: body } });
		await user.tab();
		await user.keyboard('{Enter}');
		expect(screen.getByText('Hidden field')).toBeInTheDocument();
		await user.keyboard(' ');
		expect(screen.queryByText('Hidden field')).not.toBeInTheDocument();
	});

	it('starts open and shows the hint', () => {
		render(Disclosure, { props: { label: 'More', hint: 'all', open: true, children: body } });
		expect(screen.getByRole('button', { name: /More/ })).toHaveAttribute('aria-expanded', 'true');
		expect(screen.getByText('— all')).toBeInTheDocument();
	});
});
