import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import Combobox from './Combobox.svelte';

const options = [
	{ value: 'a', label: 'srv-db-01' },
	{ value: 'b', label: 'srv-db-02' },
	{ value: 'c', label: 'win-fs01' }
];

describe('Combobox', () => {
	it('filters the options while typing and selects one', async () => {
		const user = userEvent.setup();
		render(Combobox, { props: { label: 'Host', options, value: '' } });

		const input = screen.getByRole('combobox', { name: 'Host' });
		await user.click(input);
		await user.type(input, 'win');
		expect(await screen.findByText('win-fs01')).toBeInTheDocument();
		expect(screen.queryByText('srv-db-01')).not.toBeInTheDocument();

		await user.click(screen.getByText('win-fs01'));
		expect(input).toHaveValue('win-fs01');
	});

	it('shows the empty label when nothing matches', async () => {
		const user = userEvent.setup();
		render(Combobox, { props: { label: 'Host', options, value: '', emptyLabel: 'Nothing' } });

		const input = screen.getByRole('combobox', { name: 'Host' });
		await user.click(input);
		await user.type(input, 'zzz');
		expect(await screen.findByText('Nothing')).toBeInTheDocument();
	});
});
