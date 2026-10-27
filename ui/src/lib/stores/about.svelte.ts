/*
 * The About data of the control plane: the version, the revision, the protocol versions and, for
 * the Platform Admin, the tenancy mode and the database schema level.
 *
 * The account menu shows the version in its tooltip, so the application reads `/api/v1/about`
 * once per session; the About page waits for the same read.
 */
import { service } from '#lib/api/index.ts';
import type { About } from '#lib/api/index.ts';

let about = $state<About | null>(null);
let loaded = $state(false);
let pending: Promise<void> | null = null;

export function getAbout(): About | null {
	return about;
}

/** Reads the About data once. A call during a read returns the same promise. */
export function loadAbout(): Promise<void> {
	if (loaded) return Promise.resolve();
	pending ??= service
		.about()
		.then((value) => {
			about = value;
			loaded = true;
		})
		.catch(() => {
			// The menu shows no version and the page shows its error while the read fails.
			about = null;
		})
		.finally(() => {
			pending = null;
		});
	return pending;
}

/** Drops the data of the ended session, so the next account reads its own. */
export function forgetAbout(): void {
	about = null;
	loaded = false;
	pending = null;
}
