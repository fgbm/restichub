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

export type AgentListFacets = components['schemas']['AgentListFacets'];

/** The connection state of a host in the list (FR-UI-01). */
export type HostState = 'online' | 'offline' | 'waiting' | 'revoked';

/** The filters, the order and the page of the host list (FR-UI-01). A list holds comma values. */
export interface AgentQuery {
	q?: string;
	state?: string;
	os?: string;
	agent_version?: string;
	label?: string;
	server_scripts?: 'enabled' | 'disabled';
	sort?: 'created_at' | 'hostname' | 'last_seen_at' | 'agent_version' | 'os';
	order?: 'asc' | 'desc';
	offset?: number;
	limit?: number;
	/** Adds the state counts and the filter values. */
	facets?: boolean;
}

export type RenameAgentRequest = components['schemas']['RenameAgentRequest'];

export type ReplaceAgentResponse = components['schemas']['ReplaceAgentResponse'];

export type EnrollmentToken = components['schemas']['EnrollmentTokenView'];

export interface EnrollmentTokenQuery {
	/** Only the tokens that can still enroll a host. */
	pending?: boolean;
}

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

/** The maintenance task of a repository schedule (FR-RP-08). */
export type MaintenanceTask = components['schemas']['MaintenanceTask'];

/** The maintenance schedules and the latest maintenance jobs of a repository (FR-RP-08). */
export type Maintenance = components['schemas']['MaintenanceView'];

export type MaintenanceSchedule = components['schemas']['MaintenanceScheduleView'];

export type UpdateMaintenanceRequest = components['schemas']['UpdateMaintenanceRequest'];

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
	/** Reads the lines of one step only (FR-HK-11). */
	step_id?: string;
}

/** One page of the job list, newest first (FR-SC-09). */
export type JobListResponse = components['schemas']['JobListResponse'];

/** A state tab of the job list: the server counts each one (FR-UI-01). */
export type JobGroup = Exclude<keyof components['schemas']['JobGroupCounts'], 'all'>;

/** The filters, the order and the page of the job list (FR-SC-09). A list holds comma values. */
export interface JobQuery {
	status?: JobStatus;
	plan_id?: string;
	repository_id?: string;
	agent_id?: string;
	kind?: string;
	group?: JobGroup;
	/** A part of the plan, host or repository name. */
	q?: string;
	/** RFC 3339 times of the first and the last creation time. */
	from?: string;
	to?: string;
	sort?: 'created_at' | 'started_at' | 'finished_at' | 'duration' | 'status' | 'plan' | 'host';
	order?: 'asc' | 'desc';
	cursor?: string;
	offset?: number;
	limit?: number;
	/** Asks for the counts of the state tabs. */
	counts?: boolean;
}

/** The optional agent that runs a repository job. An absent value uses the maintenance agent. */
export type JobActionRequest = components['schemas']['JobActionRequest'];
export type UnlockAllRequest = components['schemas']['UnlockAllRequest'];

/** How far a snapshot list or a tree page request is (FR-RS-01, FR-RS-02). */
export type BrowseState = components['schemas']['BrowseState'];

/** One snapshot of a repository (FR-RS-01). */
export type SnapshotItem = components['schemas']['SnapshotItem'];

/** A snapshot list with the state of its job (FR-RS-01). */
export type SnapshotsView = components['schemas']['SnapshotsView'];

/** The filters of a snapshot list request. The date range goes to the read, not here. */
export type SnapshotsQuery = components['schemas']['SnapshotsQuery'];

/** The job and the date range of a snapshot list read. */
export interface SnapshotsParams {
	job?: string;
	from?: string;
	to?: string;
}

export type NodeKind = components['schemas']['NodeKind'];

/** One entry of a snapshot tree (FR-RS-02). */
export type NodeItem = components['schemas']['NodeItem'];

/** One page of the entries of a directory in a snapshot (FR-RS-02). */
export type LsView = components['schemas']['LsView'];

export type LsQueryRequest = components['schemas']['LsQueryRequest'];

/** The policy for a file that exists in the restore target (FR-RS-04). */
export type OverwriteMode = components['schemas']['OverwriteMode'];

/** The request of a restore (FR-RS-03, FR-RS-04). */
export type RestoreRequest = components['schemas']['RestoreBody'];

/** The paths of a profile for each operating system (FR-PF-01). */
export type ProfilePaths = components['schemas']['ProfilePaths'];

/** The backup options of a profile (FR-PF-01). */
export type ProfileBackupOptions = components['schemas']['BackupOptions'];

/** Position of a script in the job (FR-HK-05). */
export type HookPhase = components['schemas']['HookPhase'];

/** The program that runs a script (FR-HK-09). */
export type ScriptInterpreter = components['schemas']['Interpreter'];

/** What a failed script does to the job (FR-HK-09). */
export type ScriptFailurePolicy = components['schemas']['FailurePolicy'];

export type ScriptEnvKind = components['schemas']['EnvKind'];

/** One environment variable of a script. A secret reference holds a secret identifier. */
export type ScriptEnv = components['schemas']['ScriptEnvDef'];

/** A script of a profile or of a job template (FR-HK-09). */
export type ScriptDef = components['schemas']['ScriptDef'];

export type ScriptParameterKind = components['schemas']['TemplateParameterKind'];

/** A declared parameter of a script template. It becomes an environment variable. */
export type ScriptTemplateParameter = components['schemas']['ScriptTemplateParameter'];

/** A script template of the tenant or a built-in one (FR-HK-12). */
export type ScriptTemplate = components['schemas']['ScriptTemplateView'];

export type ScriptTemplateListResponse = components['schemas']['ScriptTemplateListResponse'];

export type CreateScriptTemplateRequest = components['schemas']['CreateScriptTemplateRequest'];

export type UpdateScriptTemplateRequest = components['schemas']['UpdateScriptTemplateRequest'];

/** A profile template: an ordered set of scripts (FR-HK-12). */
export type JobTemplate = components['schemas']['JobTemplateView'];

export type JobTemplateListResponse = components['schemas']['JobTemplateListResponse'];

export type CreateJobTemplateRequest = components['schemas']['CreateJobTemplateRequest'];

export type UpdateJobTemplateRequest = components['schemas']['UpdateJobTemplateRequest'];

/** Whether a host trusts scripts from the control plane (FR-HK-08). */
export type ServerScriptsState = components['schemas']['ServerScriptsState'];

/** The planned step of a job with its kind, name and phase (FR-HK-11). */
export type PlannedStep = components['schemas']['PlannedStepView'];

/** The metadata of a secret. It never holds the value. */
export type Secret = components['schemas']['SecretView'];

export type SecretUsage = components['schemas']['SecretUsageView'];

export type SecretUsageKind = components['schemas']['SecretUsageKind'];

/** The records that block a delete, from a usage route or a refused delete. */
export type Blocker = components['schemas']['Blocker'];

export type BlockerKind = components['schemas']['BlockerKind'];

export type Blockers = components['schemas']['Blockers'];

export type UsageView = components['schemas']['UsageView'];

export type SecretListResponse = components['schemas']['SecretListResponse'];

export type CreateSecretRequest = components['schemas']['CreateSecretRequest'];

export type GroupBy = components['schemas']['GroupBy'];

export type SnapshotFilter = components['schemas']['SnapshotFilter'];

export type Retention = components['schemas']['Retention'];

export type CheckOptions = components['schemas']['CheckOptions'];

/** One profile (FR-PF-01). It holds no secret. */
export type Profile = components['schemas']['ProfileView'];

/** The kind of a profile parameter: a text or a reference to a secret (FR-PF-09). */
export type ParameterKind = components['schemas']['ParameterKind'];

/** One declared parameter of a profile, as a reader sees it (FR-PF-09, FR-PF-12). */
export type ProfileParameter = components['schemas']['ParameterDefView'];

/** One value of a parameter in a plan request or in a host request (FR-PF-10). */
export type ParameterValueDef = components['schemas']['ParameterValueDef'];

/** The value of one parameter on a host, as a reader sees it (FR-PF-12). */
export type ParameterValueView = components['schemas']['ParameterValueView'];

/** One override of a plan, as a reader sees it (FR-PF-10, FR-PF-12). */
export type PlanParameterValue = components['schemas']['PlanParameterValueView'];

/** One row of the parameter table of a host (FR-PF-10, FR-PF-12). */
export type HostParameterRow = components['schemas']['HostParameterView'];

/** The parameter table of a host (FR-PF-10). */
export type HostParameters = components['schemas']['HostParametersResponse'];

export type JobLimitDefaults = components['schemas']['JobLimitDefaults'];
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
	/** A part of the action code, the object name or the actor name. */
	q?: string;
	cursor?: string;
	offset?: number;
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

export type PlatformUserRecord = components['schemas']['PlatformUserRecordView'];

export type PlatformMembership = components['schemas']['PlatformMembershipView'];

export type PlatformUserListResponse = components['schemas']['PlatformUserListResponse'];

export type CreatePlatformUserRequest = components['schemas']['CreateAccountRequest'];

/** One membership of the account create request. */
export type MembershipInput = components['schemas']['MembershipInput'];

export type SetRoleRequest = components['schemas']['SetRoleRequest'];

/** An absent field leaves the value unchanged. */
export type UpdateAccountRequest = components['schemas']['UpdateAccountRequest'];

/** The kind of an alert (FR-MN-03). */
export type AlertKind = components['schemas']['AlertKind'];

export type AlertStatus = components['schemas']['AlertStatus'];

export type Alert = components['schemas']['AlertView'];

export type AlertListResponse = components['schemas']['AlertListResponse'];

/** The setting of one alert kind (FR-MN-03). */
export type AlertRule = components['schemas']['AlertRuleView'];

export type UpdateAlertRulesRequest = components['schemas']['UpdateAlertRulesRequest'];

export interface AlertQuery {
	status?: 'open' | 'resolved' | 'all';
	/** A comma list of kinds. */
	kind?: string;
	q?: string;
	cursor?: string;
	offset?: number;
	limit?: number;
}

/** A notification channel (FR-MN-04). It holds no signing secret. */
export type NotificationChannel = components['schemas']['ChannelView'];

export type ChannelKind = components['schemas']['ChannelKind'];

export type ChannelList = components['schemas']['ChannelListResponse'];

/** The signing secret of a new webhook appears one time only. */
export type ChannelSaved = components['schemas']['ChannelResponse'];

export type CreateChannelRequest = components['schemas']['CreateChannelRequest'];

export type UpdateChannelRequest = components['schemas']['UpdateChannelRequest'];

export type ChannelTestResult = components['schemas']['TestResponse'];

/** The overview of the organization (FR-MN-01). */
export type Dashboard = components['schemas']['DashboardResponse'];

export type DashboardPlan = components['schemas']['DashboardPlan'];

export type RpoState = components['schemas']['RpoState'];

export type LockState = components['schemas']['LockState'];
