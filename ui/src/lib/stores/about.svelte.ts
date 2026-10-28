/*
 * The About data of the control plane: the version, the revision, the protocol versions, the
 * environment of the server and, for the Platform Admin, the database schema level (ADR-0054,
 * ADR-0055).
 *
 * The account menu shows the version at the right end of its label row, so the application reads
 * `/api/v1/about` once per session without a block of the navigation. The About page waits for the
 * same read. A failed read stays failed until the page asks again.
 */
import { service } from '#lib/api/index.ts';
import type { About } from '#lib/api/index.ts';

let about = $state<About | null>(null);
let failed = $state(false);
let pending: Promise<void> | null = null;

export function getAbout(): About | null {
	return about;
}

export function isAboutFailed(): boolean {
	return failed;
}

/** Reads the data once. A call during a read returns the same promise. */
export function loadAbout(): Promise<void> {
	if (about || failed) return Promise.resolve();
	return read();
}

/** Reads the data again, after the person asked for it on the About page. */
export function retryAbout(): Promise<void> {
	failed = false;
	return read();
}

function read(): Promise<void> {
	pending ??= service
		.about()
		.then((value) => {
			about = value;
		})
		.catch(() => {
			about = null;
			failed = true;
		})
		.finally(() => {
			pending = null;
		});
	return pending;
}

/** Drops the data of the ended session, so the next account reads its own. */
export function forgetAbout(): void {
	about = null;
	failed = false;
	pending = null;
}
