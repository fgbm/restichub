import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { fireEvent } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import type { JobLogLine } from '#lib/api/index.ts';
import StepLog from './StepLog.svelte';

function line(seq: number): JobLogLine {
	return {
		seq,
		step_id: 'backup',
		stream: 'stdout',
		level: 'info',
		ts: '2026-10-14T10:01:02Z',
		text: `line ${seq}`
	};
}

function geometry(view: HTMLElement, height: number, top: number): void {
	Object.defineProperty(view, 'scrollHeight', { configurable: true, value: height });
	Object.defineProperty(view, 'clientHeight', { configurable: true, value: 300 });
	view.scrollTop = top;
}

describe('the log of a step', () => {
	it('follows the end of a running step and stops when the person scrolls up', async () => {
		const user = userEvent.setup();
		const props = { stepId: 'backup', name: 'Backup', live: true, onCopy: vi.fn() };
		const { rerender } = render(StepLog, { ...props, lines: [line(1)] });
		const view = screen.getByRole('log');
		const follow = screen.getByRole('button', { name: 'Follow' });
		expect(follow).toHaveAttribute('aria-pressed', 'true');

		geometry(view, 900, 0);
		await rerender({ ...props, lines: [line(1), line(2)] });
		await waitFor(() => expect(view.scrollTop).toBe(900));

		geometry(view, 900, 100);
		await fireEvent.scroll(view);
		expect(follow).toHaveAttribute('aria-pressed', 'false');

		geometry(view, 1000, 100);
		await rerender({ ...props, lines: [line(1), line(2), line(3)] });
		await Promise.resolve();
		expect(view.scrollTop).toBe(100);

		await user.click(follow);
		expect(follow).toHaveAttribute('aria-pressed', 'true');
		await waitFor(() => expect(view.scrollTop).toBe(1000));
	});

	it('offers no follow button for a step that is not running', () => {
		render(StepLog, { stepId: 'backup', name: 'Backup', lines: [line(1)], onCopy: vi.fn() });
		expect(screen.queryByRole('button', { name: 'Follow' })).not.toBeInTheDocument();
	});
});
