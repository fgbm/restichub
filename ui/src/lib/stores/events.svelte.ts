/*
 * The event stream of the session. The app layout starts it after sign-in and stops it at sign-out.
 * The mock service has no server, so it starts no stream and the screens keep their fast poll.
 */
import { ForbiddenError, UnauthorizedError, service, useMock } from '#lib/api/index.ts';
import { EventStream } from '#lib/events.ts';
import type { Listener } from '#lib/events.ts';

let connected = $state(false);

const stream = new EventStream(
	(url) => new EventSource(url, { withCredentials: true }),
	(value) => {
		connected = value;
	},
	// The API client sends the browser to the sign-in page on HTTP 401.
	async () => {
		try {
			await service.currentUser();
		} catch (error) {
			if (error instanceof UnauthorizedError || error instanceof ForbiddenError) return 'denied';
		}
		return 'ok';
	}
);

export function isStreamConnected(): boolean {
	return connected;
}

export function startEvents(): void {
	if (useMock || typeof EventSource === 'undefined') return;
	stream.start();
}

export function stopEvents(): void {
	stream.stop();
}

export function onEvent(listener: Listener): () => void {
	return stream.subscribe(listener);
}
