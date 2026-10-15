/*
 * The client of the event stream (FR-UI-02, ADR-0009). One EventSource serves the whole session.
 *
 * The stream holds identifiers and a status only. A screen reads the REST API again after an
 * event. The control plane keeps no history, so the first event of every connection is `resync`
 * and a screen reads its data again.
 */
import { API_PREFIX, endpoints } from '#lib/api/endpoints.ts';

export const EVENTS_PATH = `${API_PREFIX}${endpoints.events}`;

export const EVENT_TYPES = [
	'resync',
	'job.progress',
	'job.status',
	'job.log',
	'host.online',
	'host.offline',
	'alert.changed'
] as const;

export type EventType = (typeof EVENT_TYPES)[number];

export interface StreamEvent {
	type: EventType;
	job_id?: string;
	agent_id?: string;
	status?: string;
}

export type Listener = (event: StreamEvent) => void;

/** The part of `EventSource` that the client uses, so a test can replace it. */
export interface EventSourceLike {
	readyState: number;
	onopen: ((event: Event) => void) | null;
	onerror: ((event: Event) => void) | null;
	addEventListener(type: string, listener: (event: { data: string }) => void): void;
	close(): void;
}

export type EventSourceFactory = (url: string) => EventSourceLike;

/**
 * Asks the server whether the session is still valid. `denied` is HTTP 401 or 403: a new stream
 * would fail the same way. The browser hides the status of a failed stream, so a request tells it.
 */
export type SessionProbe = () => Promise<'ok' | 'denied'>;

const CLOSED = 2;
const FIRST_RETRY_MS = 2000;
const MAX_RETRY_MS = 30_000;

export class EventStream {
	private source: EventSourceLike | null = null;
	private listeners = new Set<Listener>();
	private retry: ReturnType<typeof setTimeout> | null = null;
	private delay = FIRST_RETRY_MS;
	private running = false;
	connected = false;

	constructor(
		private readonly create: EventSourceFactory,
		private readonly onState: (connected: boolean) => void = () => {},
		private readonly probe: SessionProbe = async () => 'ok'
	) {}

	subscribe(listener: Listener): () => void {
		this.listeners.add(listener);
		return () => this.listeners.delete(listener);
	}

	start(): void {
		if (this.running) return;
		this.running = true;
		this.open();
	}

	stop(): void {
		this.running = false;
		if (this.retry) clearTimeout(this.retry);
		this.retry = null;
		this.source?.close();
		this.source = null;
		this.setConnected(false);
	}

	private open(): void {
		const source = this.create(EVENTS_PATH);
		this.source = source;
		source.onopen = () => {
			this.delay = FIRST_RETRY_MS;
			this.setConnected(true);
		};
		source.onerror = () => {
			this.setConnected(false);
			// The browser reconnects by itself while the state is not closed. A refusal such as
			// HTTP 429 closes the source, so the client opens a new one after a growing delay.
			if (source.readyState === CLOSED && this.running) this.scheduleReopen();
		};
		for (const type of EVENT_TYPES) {
			source.addEventListener(type, (message) => this.dispatch(type, message.data));
		}
	}

	private scheduleReopen(): void {
		if (this.retry) return;
		this.source?.close();
		this.retry = setTimeout(() => void this.reopen(), this.delay);
		this.delay = Math.min(this.delay * 2, MAX_RETRY_MS);
	}

	private async reopen(): Promise<void> {
		this.retry = null;
		if (!this.running) return;
		const verdict = await this.probe().catch(() => 'ok' as const);
		if (!this.running) return;
		if (verdict === 'denied') {
			this.stop();
			return;
		}
		this.open();
	}

	private dispatch(type: EventType, data: string): void {
		let body: Partial<StreamEvent> = {};
		try {
			body = JSON.parse(data) as Partial<StreamEvent>;
		} catch {
			// An event with no readable data still tells that something changed.
		}
		const event: StreamEvent = { ...body, type };
		for (const listener of this.listeners) listener(event);
	}

	private setConnected(value: boolean): void {
		if (this.connected === value) return;
		this.connected = value;
		this.onState(value);
	}
}

/**
 * Runs `run` at most once for a burst of calls. The first call after a pause waits `wait` ms, so a
 * burst of progress events gives one read. A call during a run schedules one more run.
 */
export function coalesce(run: () => Promise<void> | void, wait: number): () => void {
	let timer: ReturnType<typeof setTimeout> | null = null;
	let running = false;
	let again = false;

	const fire = async (): Promise<void> => {
		timer = null;
		running = true;
		try {
			await run();
		} finally {
			running = false;
			if (again) {
				again = false;
				schedule();
			}
		}
	};

	const schedule = (): void => {
		if (running) {
			again = true;
			return;
		}
		if (timer) return;
		timer = setTimeout(() => void fire(), wait);
	};
	return schedule;
}

/**
 * Reads again after an event, at most once for a burst. An event that arrives while the first load
 * runs is not lost: `loaded()` starts one more read for it.
 */
export function eventReader(
	isLoading: () => boolean,
	read: () => Promise<void> | void,
	wait: number
): { notify: () => void; loaded: () => void } {
	let missed = false;
	const soon = coalesce(() => {
		if (isLoading()) {
			missed = true;
			return;
		}
		return read();
	}, wait);
	return {
		notify: soon,
		loaded: () => {
			if (!missed) return;
			missed = false;
			soon();
		}
	};
}
