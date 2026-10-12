/*
 * PLACEHOLDER FILE. `npm run gen:api` replaces this file.
 *
 * The script generates these types from the OpenAPI document of the control
 * plane (`/api/v1/openapi.json`) with `openapi-typescript`. The document needs a
 * running control plane and a database, so this file holds hand-written types
 * for the current API surface until issue #18, #19 and #20 are complete.
 *
 * The generated file keeps the `paths` and `components` shape. Read the types
 * with `components['schemas']['<Name>']`.
 */

export interface components {
	schemas: {
		/** RFC 9457 problem document (wire shape). */
		Problem: Problem;
		/** One audit record. */
		AuditRecord: AuditRecord;
		/** One page of audit records. */
		AuditList: AuditList;
	};
}

export interface paths {
	'/api/v1/audit': {
		parameters: {
			query?: AuditQuery;
		};
		get: {
			responses: {
				200: {
					content: {
						'application/json': AuditList;
					};
				};
			};
		};
	};
}

/** RFC 9457 problem document as it arrives on the wire. */
export interface Problem {
	type?: string;
	title: string;
	status: number;
	detail?: string;
	instance?: string;
	/** Stable machine-readable code from the control plane. */
	code: string;
	/** Correlation id for support. */
	request_id?: string;
}

/** One audit record. */
export interface AuditRecord {
	id: string;
	occurred_at: string;
	actor_type: 'user' | 'token' | 'system';
	actor_id: string;
	source_ip: string;
	action: string;
	object_type: string;
	object_id: string;
	/** Field changes. A secret field holds `<redacted>`. */
	diff: Record<string, { old: unknown; new: unknown }> | null;
}

/** One page of audit records with a cursor for the next page. */
export interface AuditList {
	items: AuditRecord[];
	next_cursor: string | null;
}

/** Query parameters of the audit endpoint. */
export interface AuditQuery {
	from?: string;
	to?: string;
	action?: string;
	actor_type?: string;
	actor_id?: string;
	object_type?: string;
	object_id?: string;
	cursor?: string;
	limit?: number;
}
