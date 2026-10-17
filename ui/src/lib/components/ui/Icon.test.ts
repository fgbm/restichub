import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import { parseSprite } from '../../../../scripts/sprite.mjs';
import Icon from './Icon.svelte';
import { icons } from './icons';

describe('Icon', () => {
	it('is decorative by default', () => {
		const { container } = render(Icon, { props: { name: 'host' } });
		const svg = container.querySelector('svg');
		expect(svg).toHaveAttribute('aria-hidden', 'true');
		expect(svg).not.toHaveAttribute('role');
		expect(svg).toHaveClass('icon', 'icon--md');
	});

	it('takes a label as its accessible name', () => {
		render(Icon, { props: { name: 'alert', label: 'Warning' } });
		const svg = screen.getByRole('img', { name: 'Warning' });
		expect(svg).not.toHaveAttribute('aria-hidden');
	});

	it('applies the size class', () => {
		const { container } = render(Icon, { props: { name: 'x', size: 'xs' } });
		expect(container.querySelector('svg')).toHaveClass('icon--xs');
	});

	it('turns a spinner in an HTML wrapper, so the compositor can run it', () => {
		const { container } = render(Icon, { props: { name: 'loader-circle', spin: true } });
		const svg = container.querySelector('svg');
		expect(svg?.parentElement?.tagName).toBe('SPAN');
		expect(svg?.parentElement).toHaveClass('spin');
	});

	it('draws the shapes of the symbol in the SVG namespace', () => {
		const { container } = render(Icon, { props: { name: 'job' } });
		const shapes = container.querySelectorAll('svg > *');
		expect([...shapes].map((shape) => shape.tagName)).toEqual(['path', 'circle']);
		expect(shapes[1].namespaceURI).toBe('http://www.w3.org/2000/svg');
		expect(shapes[0]).toHaveAttribute('d', (icons.job[0][1] as { d: string }).d);
	});

	it('matches the mockup sprite', () => {
		const sprite = readFileSync(join(process.cwd(), 'mockups', 'icons.svg'), 'utf8');
		expect(icons).toEqual(parseSprite(sprite));
		expect(Object.keys(icons)).toHaveLength(103);
	});
});
