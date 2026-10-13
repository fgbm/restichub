import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import Skeleton from './Skeleton.svelte';

describe('Skeleton', () => {
	it('renders the variant class and is hidden from assistive technology', () => {
		const { container } = render(Skeleton, { props: { variant: 'avatar' } });
		const skeleton = container.querySelector('.skeleton');
		expect(skeleton).toHaveClass('skeleton--avatar');
		expect(skeleton).toHaveAttribute('aria-hidden', 'true');
	});
});
