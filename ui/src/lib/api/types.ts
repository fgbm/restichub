/*
 * Domain types of the UI.
 *
 * The types come from the generated OpenAPI schema (`schema.d.ts`, built by
 * `npm run gen:api`). The generated `role`, `status`, `purpose` and audit
 * `action` fields are already closed unions, so the UI re-exports them. The
 * generated audit `diff` is an opaque object, so the UI replaces it with its own
 * diff shape.
 */
import type { components } from './schema';

export type Role = components['schemas']['Role'];

export type TenancyMode = components['schemas']['TenancyMode'];

export type Instance = components['schemas']['InstanceResponse'];

/**
 * The roles that a new user or API token may hold.
 *
 * No membership holds `platform_admin`; it is an account right (ADR-0029).
 */
export function assignableRoles(): Role[] {
	return ['admin', 'operator', 'viewer'];
}

export type AuditActorType = 'user' | 'api_token' | 'agent' | 'system';

export interface AuditChange {
	old: unknown;
	new: unknown;
}

/**
 * The field changes of one audit record.
 *
 * An update holds one entry for each changed field (`{ field: { old, new } }`).
 * A create or a delete holds one `{ old, new }` pair at the top level. The
 * component `JsonDiff` normalizes both shapes.
 */
export type AuditDiff = Record<string, unknown>;

export type User = components['schemas']['UserView'];

export type Me = components['schemas']['MeResponse'];

export type SetupStatus = components['schemas']['SetupStatusResponse'];

export type SignInRequest = components['schemas']['SignInRequest'];

/** One membership of the caller (ADR-0025). */
export type TenantAccess = components['schemas']['TenantAccessView'];

export type TenantAccessListResponse = components['schemas']['TenantAccessListResponse'];

export type SwitchTenantRequest = components['schemas']['SwitchTenantRequest'];

export type SetupRequest = components['schemas']['SetupRequest'];

export type ChangePasswordRequest = components['schemas']['ChangePasswordRequest'];

export type CreateUserRequest = components['schemas']['CreateUserRequest'];

/** An absent field does not change the value. */
export type UpdateUserRequest = components['schemas']['UpdateUserRequest'];

export type ResetPasswordRequest = components['schemas']['ResetPasswordRequest'];

export type UserListResponse = components['schemas']['UserListResponse'];

export type ApiToken = components['schemas']['ApiTokenView'];

/** The value appears one time only. */
export type ApiTokenCreated = components['schemas']['CreateTokenResponse'];

export type CreateApiTokenRequest = components['schemas']['CreateTokenRequest'];

export type ApiTokenListResponse = components['schemas']['ApiTokenListResponse'];

/** One enrolled agent (ADR-0026). */
export type Agent = components['schemas']['AgentView'];

export type AgentStatus = components['schemas']['AgentStatus'];

export type AgentListResponse = components['schemas']['AgentListResponse'];

export type RenameAgentRequest = components['schemas']['RenameAgentRequest'];

export type EnrollmentToken = components['schemas']['EnrollmentTokenView'];

/** The value appears one time only. */
export type EnrollmentTokenCreated = components['schemas']['CreateEnrollmentTokenResponse'];

export type CreateEnrollmentTokenRequest = components['schemas']['CreateEnrollmentTokenRequest'];

export type EnrollmentTokenListResponse = components['schemas']['EnrollmentTokenListResponse'];

/** The backend settings of a repository (FR-RP-01). */
export type BackendSettings = components['schemas']['BackendSettings'];

export type RepositoryBackendKind = BackendSettings['kind'];

/** The presence of the bound secrets. It never holds a value (AC-RP-02). */
export type RepositoryCredentials = components['schemas']['CredentialsView'];

/** One repository (FR-RP-01). */
export type Repository = components['schemas']['RepositoryView'];

export type RepositoryListResponse = components['schemas']['RepositoryListResponse'];

export type CreateRepositoryRequest = components['schemas']['CreateRepositoryRequest'];

export type UpdateRepositoryRequest = components['schemas']['UpdateRepositoryRequest'];

/** The kind of a job (FR-PR-02). */
export type JobKind = components['schemas']['JobKind'];

/** The lifecycle status of a job (FR-SC-03). */
export type JobStatus = components['schemas']['JobStatus'];

/** The typed result of a finished job (FR-AG-09). */
export type JobResultCode = components['schemas']['JobResultCode'];

/** One job with its lifecycle (FR-PR-01). */
export type Job = components['schemas']['JobView'];

/** One job with its steps, summary and progress (FR-MN-02). */
export type JobDetail = components['schemas']['JobDetailView'];

/** One step of a job with its typed result (FR-AG-09). */
export type JobStep = components['schemas']['StepView'];

/** The output of a step: the typed summary that restic reported (FR-MN-02). */
export type StepOutput = components['schemas']['StepOutput'];

/** The last progress of a running job (FR-AG-08). */
export type JobProgress = components['schemas']['ProgressView'];

/** One stored line of a job log (FR-LG-01). */
export type JobLogLine = components['schemas']['LogLineView'];

/** One page of log lines (FR-LG-01). */
export type JobLogPage = components['schemas']['LogPageView'];

/** The position and the size of a log page request. */
export interface JobLogQuery {
	after_seq?: number;
	limit?: number;
}

/** One page of the job list, newest first (FR-SC-09). */
export type JobListResponse = components['schemas']['JobListResponse'];

/** The filters and the cursor of the job list (FR-SC-09). */
export interface JobQuery {
	status?: JobStatus;
	plan_id?: string;
	repository_id?: string;
	agent_id?: string;
	cursor?: string;
	limit?: number;
}

/** The optional agent that runs a repository job. An absent value uses the maintenance agent. */
export type JobActionRequest = components['schemas']['JobActionRequest'];

/** The paths of a profile for each operating system (FR-PF-01). */
export type ProfilePaths = components['schemas']['ProfilePaths'];

/** The backup options of a profile (FR-PF-01). */
export type ProfileBackupOptions = components['schemas']['BackupOptions'];

/** Position of a hook in the job (FR-HK-05). */
export type HookPhase = components['schemas']['HookPhase'];

export type HookParameter = components['schemas']['HookParameter'];

export type HookRef = components['schemas']['HookRef'];

export type GroupBy = components['schemas']['GroupBy'];

export type SnapshotFilter = components['schemas']['SnapshotFilter'];

export type Retention = components['schemas']['Retention'];

export type CheckOptions = components['schemas']['CheckOptions'];

/** One profile (FR-PF-01). It holds no secret. */
export type Profile = components['schemas']['ProfileView'];

export type ProfileListResponse = components['schemas']['ProfileListResponse'];

export type CreateProfileRequest = components['schemas']['CreateProfileRequest'];

export type UpdateProfileRequest = components['schemas']['UpdateProfileRequest'];

/** A warning that the save-time compile reported (FR-PF-05). */
export type ProfileWarning = components['schemas']['ProfileWarning'];

/** The result of a profile create or update: the profile and the save-time warnings. */
export type ProfileResponse = components['schemas']['ProfileResponse'];

/** One previewed step with masked secrets (FR-PF-04). */
export type PreviewStep = components['schemas']['PreviewStepView'];

export type PreviewResponse = components['schemas']['PreviewResponse'];

export type PreviewRequest = components['schemas']['PreviewRequest'];

export type PreviewEnvVar = components['schemas']['PreviewEnvView'];

/** The missed run policy of a plan (FR-SC-07). */
export type MissedRunPolicy = components['schemas']['MissedRunPolicy'];

/** One plan: a profile bound to an agent with a schedule (FR-SC-01). */
export type Plan = components['schemas']['PlanView'];

export type PlanListResponse = components['schemas']['PlanListResponse'];

export type CreatePlanRequest = components['schemas']['CreatePlanRequest'];

export type UpdatePlanRequest = components['schemas']['UpdatePlanRequest'];

/** The result of a plan create or update: the plan. */
export type PlanResponse = components['schemas']['PlanResponse'];

/** The next fire times of a plan, in UTC (FR-SC-01). */
export type NextRunsResponse = components['schemas']['NextRunsResponse'];

/** The request body of the schedule preview (FR-SC-01). */
export type PreviewScheduleRequest = components['schemas']['PreviewScheduleRequest'];

/** The next fire times of an unsaved schedule, in UTC (FR-SC-01). */
export type SchedulePreviewResponse = components['schemas']['SchedulePreviewResponse'];

/**
 * One audit record. The generated `diff` type is an opaque object, so the UI
 * replaces it with the diff shape that the control plane writes.
 */
export type AuditRecord = Omit<components['schemas']['AuditItem'], 'diff'> & {
	diff?: AuditDiff | null;
};

export type AuditPage = Omit<components['schemas']['AuditPageResponse'], 'items'> & {
	items: AuditRecord[];
};

export interface AuditQuery {
	from?: string;
	to?: string;
	action?: string;
	actor_type?: AuditActorType;
	actor_id?: string;
	object_type?: string;
	object_id?: string;
	cursor?: string;
	limit?: number;
}

export type TenantStatus = components['schemas']['TenantStatus'];

export type Tenant = components['schemas']['PlatformTenantView'];

export type TenantListResponse = components['schemas']['TenantListResponse'];

export type CreateTenantRequest = components['schemas']['CreateTenantRequest'];

/** The derived administrator state of an organization (ADR-0028). */
export type AdminState = components['schemas']['AdminState'];

/** An installation account with its memberships (ADR-0028). */
export type PlatformUser = components['schemas']['PlatformUserView'];

export type PlatformMembership = components['schemas']['PlatformMembershipView'];

export type PlatformUserListResponse = components['schemas']['PlatformUserListResponse'];

export type CreatePlatformUserRequest = components['schemas']['CreateAccountRequest'];

/** One membership of the account create request. */
export type MembershipInput = components['schemas']['MembershipInput'];

export type SetRoleRequest = components['schemas']['SetRoleRequest'];

/** An absent field leaves the value unchanged. */
export type UpdateAccountRequest = components['schemas']['UpdateAccountRequest'];
