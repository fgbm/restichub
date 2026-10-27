import { waitFor } from '@testing-library/svelte';
import { expect } from 'vitest';

// bits-ui takes the body scroll lock of a closed layer off the body in a 24 ms timer. A click
// inside that window lands on a body with `pointer-events: none` and user-event refuses it, so
// every test that closes a layer waits for the unlock before it interacts again.
export async function bodyUnlocked(): Promise<void> {
	if (document.body.style.pointerEvents === '') return;
	await waitFor(() => expect(document.body.style.pointerEvents).toBe(''), { interval: 5 });
}
