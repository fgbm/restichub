import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import { icons } from '#lib/components/ui/icons.ts';
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

	it('draws every icon with at least one shape', () => {
		const { container } = render(ComponentGallery);
		const cells = container.querySelectorAll('[data-testid="gallery-icon"]');
		expect(cells).toHaveLength(Object.keys(icons).length);
		for (const cell of cells) {
			expect(cell.querySelectorAll('svg > *').length).toBeGreaterThan(0);
		}
	});
});
