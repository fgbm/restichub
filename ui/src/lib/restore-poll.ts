/*
 * Waits for a browse job of the Restore screen: the API answers at once with the state `pending`,
 * and the screen asks again until the job ends (FR-RS-01, FR-RS-02).
 */
import type { BrowseState } from '#lib/api/index.ts';

export const POLL_MS = 300;
export const BROWSE_TIMEOUT_MS = 90_000;

export class BrowseTimeout extends Error {
	constructor() {
		super('The browse job did not finish in time.');
		this.name = 'BrowseTimeout';
	}
}

interface Options {
	/** Returns true when the screen no longer wants the result, such as after a new request. */
	cancelled?: () => boolean;
	interval?: number;
	timeout?: number;
}

/**
 * Follows a request until its state is not `pending`. Returns `null` when the caller cancelled and
 * throws [`BrowseTimeout`] when the job takes too long.
 */
export async function settle<T extends { state: BrowseState }>(
	first: T,
	read: (view: T) => Promise<T>,
	options: Options = {}
): Promise<T | null> {
	const interval = options.interval ?? POLL_MS;
	const timeout = options.timeout ?? BROWSE_TIMEOUT_MS;
	const started = Date.now();
	let view = first;
	while (view.state === 'pending') {
		if (options.cancelled?.()) return null;
		if (Date.now() - started > timeout) throw new BrowseTimeout();
		await new Promise((resolve) => setTimeout(resolve, interval));
		if (options.cancelled?.()) return null;
		view = await read(view);
	}
	return options.cancelled?.() ? null : view;
}
