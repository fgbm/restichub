import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/svelte';
import { afterEach } from 'vitest';
import { bodyUnlocked } from './bodyLock.ts';

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

afterEach(async () => {
	cleanup();
	await bodyUnlocked();
	// A kept view must not leak from one test to the next.
	localStorage.clear();
});
