/*
 * The event stream of the session. The app layout starts it after sign-in and stops it at sign-out.
 * The mock service has no server, so it starts no stream and the screens keep their fast poll.
 */
import { useMock } from '#lib/api/index.ts';
import { EventStream } from '#lib/events.ts';
import type { Listener } from '#lib/events.ts';

let connected = $state(false);

const stream = new EventStream(
	(url) => new EventSource(url, { withCredentials: true }),
	(value) => {
		connected = value;
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
