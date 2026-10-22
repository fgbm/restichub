/*
 * One place that numbers its requests, so that a late answer of an older request does not replace
 * the newer state of the form (FR-SC-01).
 */

export interface Latest {
	/**
	 * Starts a request and gives its guard. The guard answers false after a newer request starts.
	 */
	start(): () => boolean;
}

/** Numbers the requests of one place. Only the answer of the newest request is current. */
export function latest(): Latest {
	let sequence = 0;
	return {
		start() {
			const mine = ++sequence;
			return () => mine === sequence;
		}
	};
}
