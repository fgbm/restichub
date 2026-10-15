import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { coalesce, EVENTS_PATH, EventStream } from './events.ts';
import type { EventSourceLike, StreamEvent } from './events.ts';

class FakeSource implements EventSourceLike {
	readyState = 0;
	onopen: ((event: Event) => void) | null = null;
	onerror: ((event: Event) => void) | null = null;
	closed = false;
	private handlers = new Map<string, (event: { data: string }) => void>();

	addEventListener(type: string, listener: (event: { data: string }) => void): void {
		this.handlers.set(type, listener);
	}
	close(): void {
		this.closed = true;
		this.readyState = 2;
	}
	emit(type: string, data: string): void {
		this.handlers.get(type)?.({ data });
	}
}

describe('ac_ui_02 the event stream client', () => {
	let sources: FakeSource[];
	let states: boolean[];
	let stream: EventStream;

	beforeEach(() => {
		vi.useFakeTimers();
		sources = [];
		states = [];
		stream = new EventStream(
			(url) => {
				expect(url).toBe(EVENTS_PATH);
				const source = new FakeSource();
				sources.push(source);
				return source;
			},
			(connected) => states.push(connected)
		);
	});

	afterEach(() => {
		stream.stop();
		vi.useRealTimers();
	});

	it('opens one source however often it starts', () => {
		stream.start();
		stream.start();
		expect(sources).toHaveLength(1);
	});

	it('hands a typed event to every listener', () => {
		const first: StreamEvent[] = [];
		const second: StreamEvent[] = [];
		stream.subscribe((event) => first.push(event));
		const off = stream.subscribe((event) => second.push(event));
		stream.start();
		sources[0].emit('job.status', '{"job_id":"j1","status":"running"}');
		off();
		sources[0].emit('host.online', '{"agent_id":"a1"}');
		expect(first).toEqual([
			{ type: 'job.status', job_id: 'j1', status: 'running' },
			{ type: 'host.online', agent_id: 'a1' }
		]);
		expect(second).toEqual([{ type: 'job.status', job_id: 'j1', status: 'running' }]);
	});

	it('still reports an event with unreadable data', () => {
		const seen: StreamEvent[] = [];
		stream.subscribe((event) => seen.push(event));
		stream.start();
		sources[0].emit('resync', 'not json');
		expect(seen).toEqual([{ type: 'resync' }]);
	});

	it('tracks the connection state', () => {
		stream.start();
		sources[0].onopen?.(new Event('open'));
		sources[0].onerror?.(new Event('error'));
		expect(states).toEqual([true, false]);
	});

	it('does not reopen a source that the browser reconnects by itself', () => {
		stream.start();
		sources[0].readyState = 0;
		sources[0].onerror?.(new Event('error'));
		vi.advanceTimersByTime(60_000);
		expect(sources).toHaveLength(1);
	});

	it('reopens a closed source after a growing delay', () => {
		stream.start();
		sources[0].close();
		sources[0].onerror?.(new Event('error'));
		vi.advanceTimersByTime(1999);
		expect(sources).toHaveLength(1);
		vi.advanceTimersByTime(1);
		expect(sources).toHaveLength(2);
		sources[1].close();
		sources[1].onerror?.(new Event('error'));
		vi.advanceTimersByTime(3999);
		expect(sources).toHaveLength(2);
		vi.advanceTimersByTime(1);
		expect(sources).toHaveLength(3);
	});

	it('stops for good', () => {
		stream.start();
		sources[0].onopen?.(new Event('open'));
		stream.stop();
		expect(sources[0].closed).toBe(true);
		expect(states).toEqual([true, false]);
		vi.advanceTimersByTime(60_000);
		expect(sources).toHaveLength(1);
	});
});

describe('coalesce', () => {
	beforeEach(() => vi.useFakeTimers());
	afterEach(() => vi.useRealTimers());

	it('runs once for a burst of calls', async () => {
		const run = vi.fn(async () => {});
		const soon = coalesce(run, 100);
		soon();
		soon();
		soon();
		await vi.advanceTimersByTimeAsync(100);
		expect(run).toHaveBeenCalledTimes(1);
	});

	it('runs again for a call that arrives during a run', async () => {
		let release: () => void = () => {};
		const run = vi.fn(
			() =>
				new Promise<void>((resolve) => {
					release = resolve;
				})
		);
		const soon = coalesce(run, 100);
		soon();
		await vi.advanceTimersByTimeAsync(100);
		expect(run).toHaveBeenCalledTimes(1);
		soon();
		release();
		await vi.advanceTimersByTimeAsync(100);
		expect(run).toHaveBeenCalledTimes(2);
	});
});
