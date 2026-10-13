import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import Pagination from './Pagination.svelte';

describe('Pagination', () => {
	it('disables the previous button on the first page and moves forward', async () => {
		const user = userEvent.setup();
		render(Pagination, { props: { page: 1, pageCount: 3 } });

		expect(screen.getByRole('button', { name: 'Previous' })).toBeDisabled();
		expect(screen.getByText('Page 1 of 3')).toBeInTheDocument();

		await user.click(screen.getByRole('button', { name: 'Next' }));
		expect(screen.getByText('Page 2 of 3')).toBeInTheDocument();
	});

	it('disables the next button on the last page', () => {
		render(Pagination, { props: { page: 3, pageCount: 3 } });
		expect(screen.getByRole('button', { name: 'Next' })).toBeDisabled();
	});
});
