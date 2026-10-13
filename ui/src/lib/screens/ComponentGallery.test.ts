import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import ComponentGallery from './ComponentGallery.svelte';

describe('ComponentGallery', () => {
	it('renders the sections of the gallery', () => {
		render(ComponentGallery);
		expect(screen.getByRole('heading', { level: 1, name: 'Components' })).toBeInTheDocument();
		expect(screen.getByRole('heading', { name: 'Buttons' })).toBeInTheDocument();
		expect(screen.getByRole('heading', { name: 'Table' })).toBeInTheDocument();
		expect(
			screen.getByRole('heading', { name: 'Menus, dialogs and side panel' })
		).toBeInTheDocument();
	});
});
