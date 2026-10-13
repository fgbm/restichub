import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import Checkbox from './Checkbox.svelte';

describe('Checkbox', () => {
	it('ties the label to the checkbox and toggles it', async () => {
		const user = userEvent.setup();
		render(Checkbox, { props: { label: 'Enable', checked: false } });

		const box = screen.getByLabelText('Enable') as HTMLInputElement;
		expect(box.checked).toBe(false);

		await user.click(box);
		expect(box.checked).toBe(true);
	});
});
