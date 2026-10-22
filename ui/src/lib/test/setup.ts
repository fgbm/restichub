import '@testing-library/jest-dom/vitest';
import { cleanup, waitFor } from '@testing-library/svelte';
import { afterEach, expect } from 'vitest';

// jsdom has no ResizeObserver. The headless primitives observe the size of the
// floating content, so the tests need a stub.
class ResizeObserverStub {
	observe(): void {}
	unobserve(): void {}
	disconnect(): void {}
}

if (!('ResizeObserver' in globalThis)) {
	globalThis.ResizeObserver = ResizeObserverStub as unknown as typeof ResizeObserver;
}

// bits-ui takes the scroll lock of a closed layer off the body in a 24 ms timer. A test that starts
// in that window clicks on a body with `pointer-events: none`, so each test waits for the unlock.
async function bodyUnlocked(): Promise<void> {
	await waitFor(() => expect(document.body.style.pointerEvents).toBe(''));
}

afterEach(async () => {
	cleanup();
	await bodyUnlocked();
	// A kept view must not leak from one test to the next.
	localStorage.clear();
});
