import { describe, expect, it } from 'vitest';
import { assignableRoles } from './types';

describe('assignableRoles', () => {
	it('never offers the platform_admin role (ADR-0029)', () => {
		expect(assignableRoles()).toEqual(['admin', 'operator', 'viewer']);
		expect(assignableRoles()).not.toContain('platform_admin');
	});
});
