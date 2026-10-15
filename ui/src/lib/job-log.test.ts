import { describe, expect, it } from 'vitest';
import type { JobLogLine } from '#lib/api/index.ts';
import { countLines, filterLines, groupLines, logText, scrolledAwayFromEnd } from './job-log.ts';

function line(seq: number, stepId: string, text: string, level: JobLogLine['level'] = 'info') {
	return {
		seq,
		step_id: stepId,
		stream: 'stdout',
		level,
		ts: '2026-10-14T10:01:02Z',
		text
	} as const;
}

const LINES = [
	line(1, 'script-0', 'dump started'),
	line(2, 'backup', 'scan C:\\Data', 'info'),
	line(3, 'backup', 'cannot read C:\\Data\\a.tmp', 'warn'),
	line(4, '', 'agent restarted'),
	line(5, 'gone', 'orphan line', 'error')
];

describe('groupLines', () => {
	it('keeps the order of the log inside each step', () => {
		const groups = groupLines(LINES, ['script-0', 'backup']);
		expect(groups.byStep.get('backup')?.map((item) => item.seq)).toEqual([2, 3]);
		expect(groups.byStep.get('script-0')?.map((item) => item.seq)).toEqual([1]);
	});

	it('puts lines of no step and of an unknown step into the general log', () => {
		const groups = groupLines(LINES, ['script-0', 'backup']);
		expect(groups.general.map((item) => item.seq)).toEqual([4, 5]);
	});

	it('gives an empty general log when every line has a step', () => {
		expect(groupLines(LINES.slice(0, 3), ['script-0', 'backup']).general).toEqual([]);
	});
});

describe('countLines', () => {
	it('counts the warnings and the errors', () => {
		expect(countLines(LINES)).toEqual({ total: 5, warnings: 1, errors: 1 });
	});
});

describe('filterLines', () => {
	it('returns every line for the all filter without a text', () => {
		expect(filterLines(LINES, 'all', '  ')).toHaveLength(5);
	});

	it('keeps warnings and errors only for the problems filter', () => {
		expect(filterLines(LINES, 'problems', '').map((item) => item.seq)).toEqual([3, 5]);
	});

	it('matches the text without regard to the case and combines with the level', () => {
		expect(filterLines(LINES, 'all', 'C:\\DATA').map((item) => item.seq)).toEqual([2, 3]);
		expect(filterLines(LINES, 'problems', 'c:\\data').map((item) => item.seq)).toEqual([3]);
	});
});

describe('logText', () => {
	it('writes one line per entry with the time and the level', () => {
		expect(logText(LINES.slice(2, 4))).toBe(
			'2026-10-14T10:01:02Z WARN cannot read C:\\Data\\a.tmp\n2026-10-14T10:01:02Z INFO agent restarted'
		);
	});
});

describe('scrolledAwayFromEnd', () => {
	it('stops the following when the view is scrolled up past the margin', () => {
		expect(scrolledAwayFromEnd({ scrollHeight: 1000, scrollTop: 400, clientHeight: 300 })).toBe(
			true
		);
	});

	it('keeps the following at the end of the log', () => {
		expect(scrolledAwayFromEnd({ scrollHeight: 1000, scrollTop: 690, clientHeight: 300 })).toBe(
			false
		);
		expect(scrolledAwayFromEnd({ scrollHeight: 1000, scrollTop: 700, clientHeight: 300 })).toBe(
			false
		);
	});
});
