<script lang="ts">
	/*
	 * Component gallery: every element of `ui/` in its states, in the order of
	 * ui/mockups/components.html.
	 *
	 * This is a development tool, so the labels are English and it is not part of a
	 * production screen. The route `(dev)/components` mounts it in a dev build only.
	 */
	import { icons } from '#lib/components/ui/icons.ts';
	import {
		Alert,
		Icon,
		Badge,
		Button,
		Checkbox,
		Combobox,
		ConfirmDialog,
		CopyField,
		Dialog,
		DropdownMenu,
		EmptyState,
		JsonDiff,
		KeyValue,
		ListView,
		PageHeader,
		Pagination,
		Panel,
		PasswordField,
		Popover,
		RecordPicker,
		Select,
		SidePanel,
		Skeleton,
		StatusPill,
		Switch,
		DataTable,
		FilterBar,
		Tabs,
		TextField,
		Toast,
		Tooltip
	} from '#lib/components/ui/index.ts';
	import type {
		IconName,
		ListMeta,
		MenuItem,
		RecordOption,
		StatusTone,
		TableColumn
	} from '#lib/components/ui/index.ts';
	import { iconOf, jobState, pillOf } from '#lib/vocabulary.ts';
	import type { Job } from '#lib/api/index.ts';

	let confirmOpen = $state(false);
	let destructiveOpen = $state(false);
	let formOpen = $state(false);
	let panelOpen = $state(false);
	let page = $state(1);
	let tab = $state('overview');
	let username = $state('operator');
	let displayName = $state('Boris Ivanov');
	let role = $state('operator');
	let host = $state('');
	let profile = $state('p1');
	let profileFilter = $state('');

	const profileOptions: RecordOption[] = [
		{ value: 'p1', label: 'user-profiles', description: ['test-s3-yandex', { tag: 'S3' }] },
		{ value: 'p2', label: 'db-hourly', description: ['rest-main', { tag: 'REST' }] },
		{
			value: 'p3',
			label: 'nas-files',
			description: ['nas-local', { tag: 'Local' }, 'host MINDCORE']
		}
	];

	const menuItems: MenuItem[] = [
		{ label: 'Open', icon: 'eye' },
		{ label: 'Run backup now', icon: 'job' },
		{ label: 'Refresh certificate', icon: 'refresh' },
		{ label: 'Download log', icon: 'download', disabled: true },
		{ label: 'Revoke agent', icon: 'octagon', tone: 'danger', separatorBefore: true }
	];

	const tabs = [
		{ value: 'overview', label: 'Overview', icon: 'overview' as const },
		{ value: 'jobs', label: 'Jobs', icon: 'job' as const, count: 12 },
		{ value: 'files', label: 'Files', icon: 'file' as const },
		{ value: 'settings', label: 'Settings', icon: 'settings' as const }
	];

	const roles = [
		{ value: 'operator', label: 'Operator' },
		{ value: 'viewer', label: 'Viewer' }
	];

	interface Host {
		id: string;
		name: string;
		os: string;
		status: string;
		tone: StatusTone;
		lastBackup: string;
	}

	let galleryFilters = $state<Record<string, string>>({ q: '', status: 'online' });

	const columns: TableColumn<Host>[] = [
		{ key: 'name', header: 'Host', render: hostCell },
		{ key: 'status', header: 'Status', render: statusCell },
		{ key: 'lastBackup', header: 'Last backup' }
	];
	const rows: Host[] = [
		{
			id: 'h1',
			name: 'win-fs01',
			os: 'Windows Server 2022',
			status: 'Online',
			tone: 'success',
			lastBackup: '14 minutes ago'
		},
		{
			id: 'h2',
			name: 'srv-db-01',
			os: 'Ubuntu 24.04',
			status: 'Running',
			tone: 'running',
			lastBackup: '26 minutes ago'
		},
		{
			id: 'h3',
			name: 'mac-build-01',
			os: 'macOS 14',
			status: 'Offline',
			tone: 'failed',
			lastBackup: '2 days ago'
		}
	];

	interface Run {
		id: string;
		name: string;
		job: Pick<Job, 'status' | 'result_code'>;
		meta: ListMeta[];
		duration: string;
		day: string;
	}

	let listFilters = $state<Record<string, string>>({});

	const runs: Run[] = [
		{
			id: 'r1',
			name: 'srv-db-01-postgres',
			job: { status: 'running' },
			meta: ['srv-db-01', 'today 09:40', 'Backup', 'test-s3-yandex'],
			duration: '3 min 05 s',
			day: 'Today, 16 October'
		},
		{
			id: 'r2',
			name: 'user-profiles',
			job: { status: 'failed' },
			meta: ['BUH-PC07', 'today 02:00', 'Backup', 'test-s3-yandex'],
			duration: '12 s',
			day: 'Today, 16 October'
		},
		{
			id: 'r3',
			name: 'user-profiles',
			job: { status: 'succeeded', result_code: 'succeeded_with_warnings' },
			meta: ['MINDCORE', 'today 02:00', 'Backup', 'test-s3-yandex'],
			duration: '4 min 12 s',
			day: 'Today, 16 October'
		},
		{
			id: 'r4',
			name: '1c-base-daily',
			job: { status: 'succeeded' },
			meta: ['srv-1c-01', 'yesterday 01:00', 'Backup', 'test-s3-yandex', 'manual'],
			duration: '18 min 40 s',
			day: 'Yesterday, 15 October'
		}
	];

	const record = [
		{ key: 'host', label: 'Host name', value: 'win-fs01', mono: true },
		{ key: 'agent', label: 'Agent version', value: '0.4.2 (restic 0.19.1)' },
		{ key: 'os', label: 'Operating system', value: 'Windows Server 2022 (x86_64)' },
		{ key: 'cert', label: 'Certificate expires', value: '15 Oct 2026' }
	];

	const diff = {
		disabled: { old: false, new: true },
		role: { old: 'viewer', new: 'operator' }
	};
</script>

{#snippet hostCell(row: Host)}
	<span class="strong">{row.name}</span>
	<div class="sub">{row.os}</div>
{/snippet}

{#snippet statusCell(row: Host)}
	<StatusPill tone={row.tone}>{row.status}</StatusPill>
{/snippet}

<div class="gallery">
	<PageHeader
		title="Components"
		description="Every element in its states, with the same tokens as the mockups."
	/>

	<section>
		<h2>Buttons</h2>
		<div class="demo-row">
			<div class="demo-state"><span class="demo-label">default</span><Button>Cancel</Button></div>
			<div class="demo-state">
				<span class="demo-label">disabled</span><Button disabled>Cancel</Button>
			</div>
			<div class="demo-state">
				<span class="demo-label">loading</span><Button loading>Saving</Button>
			</div>
			<div class="demo-state">
				<span class="demo-label">primary</span><Button variant="primary">Run job</Button>
			</div>
			<div class="demo-state">
				<span class="demo-label">danger</span><Button variant="danger">Revoke</Button>
			</div>
			<div class="demo-state">
				<span class="demo-label">ghost</span><Button variant="ghost">Refresh</Button>
			</div>
			<div class="demo-state">
				<span class="demo-label">link</span><Button variant="link">View log</Button>
			</div>
			<div class="demo-state">
				<span class="demo-label">small</span><Button size="sm">Copy</Button>
			</div>
		</div>

		<h3>Icon buttons</h3>
		<div class="demo-row">
			<div class="demo-state">
				<span class="demo-label">icon only</span><Button icon="refresh" aria-label="Refresh" />
			</div>
			<div class="demo-state">
				<span class="demo-label">with icon</span><Button icon="plus">Add host</Button>
			</div>
			<div class="demo-state">
				<span class="demo-label">primary with icon</span>
				<Button variant="primary" icon="job">Run job</Button>
			</div>
			<div class="demo-state">
				<span class="demo-label">small icon</span>
				<Button size="sm" icon="more" aria-label="More" />
			</div>
			<div class="demo-state">
				<span class="demo-label">small with icon</span>
				<Button size="sm" icon="eye">Details</Button>
			</div>
		</div>

		<h3>Icons</h3>
		<div class="icon-grid">
			{#each Object.keys(icons) as name (name)}
				<div class="icon-cell" data-testid="gallery-icon">
					<Icon name={name as IconName} size="lg" />
					<span class="demo-label">{name}</span>
				</div>
			{/each}
		</div>
	</section>

	<section>
		<h2>Labels and status</h2>
		<div class="demo-row">
			<Badge>Windows Server 2022</Badge>
			<Badge icon="tag">prod</Badge>
			<Badge icon="lock">Encrypted</Badge>
			<Badge tone="accent">production</Badge>
			<Badge tone="accent">daily</Badge>
		</div>
		<div class="demo-row spaced">
			<StatusPill tone="success">Success</StatusPill>
			<StatusPill tone="running">Running</StatusPill>
			<StatusPill tone="queued">Queued</StatusPill>
			<StatusPill tone="warning">Warning</StatusPill>
			<StatusPill tone="failed">Failed</StatusPill>
			<StatusPill tone="cancelled">Cancelled</StatusPill>
			<StatusPill tone="info">Online</StatusPill>
			<StatusPill tone="neutral">Revoked</StatusPill>
		</div>
	</section>

	<section>
		<h2>Forms</h2>
		<div class="grid-3">
			<div class="demo-cell">
				<span class="demo-label">Text input: default, disabled, invalid</span>
				<TextField label="Display name" value="Weekly file backup" help="Shown in the plan list." />
				<TextField value="Disabled input" aria-label="Disabled input" disabled />
				<TextField
					label="Repository name"
					value="s3 prod"
					error="Use lowercase letters, digits and dashes."
				/>
			</div>
			<div class="demo-cell">
				<span class="demo-label">Select and password</span>
				<Select
					label="Operating system"
					value="windows"
					options={[
						{ value: 'windows', label: 'Windows Server 2022' },
						{ value: 'ubuntu', label: 'Ubuntu 24.04' }
					]}
				/>
				<PasswordField label="Password" value="correct-horse-battery" />
				<Combobox
					label="Host"
					bind:value={host}
					placeholder="Search hosts"
					options={[
						{ value: 'srv-db-01', label: 'srv-db-01' },
						{ value: 'srv-db-02', label: 'srv-db-02' },
						{ value: 'win-fs01', label: 'win-fs01' }
					]}
				/>
			</div>
			<div class="demo-cell">
				<span class="demo-label">Checkbox</span>
				<Checkbox label="Unchecked" />
				<Checkbox label="Checked" checked />
				<Checkbox label="Mixed" indeterminate />
				<Checkbox label="Disabled" disabled />
				<span class="demo-label">Toggle switch</span>
				<Switch label="Enabled" checked />
				<Switch label="Off" />
				<Switch label="Disabled" disabled />
			</div>
		</div>

		<h3>Record picker</h3>
		<div class="grid-3">
			<div class="demo-cell">
				<span class="demo-label">default</span>
				<RecordPicker
					label="Profile"
					bind:value={profile}
					options={profileOptions}
					searchPlaceholder="Profile or repository"
				/>
			</div>
			<div class="demo-cell">
				<span class="demo-label">filter with All</span>
				<RecordPicker
					label="Profile"
					bind:value={profileFilter}
					options={profileOptions}
					allLabel="All"
				/>
			</div>
			<div class="demo-cell">
				<span class="demo-label">disabled empty, invalid</span>
				<RecordPicker label="Profile" options={profileOptions} disabled />
				<RecordPicker
					label="Profile"
					value="p2"
					options={profileOptions}
					error="The profile does not exist."
				/>
			</div>
		</div>
	</section>

	<section>
		<h2>Banners, alerts and toasts</h2>
		<div class="stack">
			<Alert tone="info" title="A new agent version is available">
				Version 0.4.3 updates the restic binary to 0.19.1. Apply the update from the host page.
				{#snippet actions()}
					<Button size="sm">View hosts</Button>
					<Button size="sm" icon="x" aria-label="Dismiss" />
				{/snippet}
			</Alert>
			<Alert tone="success" title="The job completed" />
			<Alert tone="attention" title="The certificate expires in 5 days">
				The agent rotates its certificate at two thirds of the lifetime.
				{#snippet actions()}
					<Button size="sm">Rotate now</Button>
				{/snippet}
			</Alert>
			<Alert tone="danger" title="The repository is locked">
				A stale lock blocks the backup. A maintenance agent can remove it after 30 minutes.
				{#snippet actions()}
					<Button size="sm">Open repository</Button>
				{/snippet}
			</Alert>
		</div>
		<div class="grid-3 spaced">
			<Alert
				variant="inline"
				tone="danger"
				title="Correct the fields and try again."
				code="validation_failed"
			/>
			<Alert variant="inline" tone="success" title="The profile is saved." />
			<Alert variant="inline" title="Log retention is 30 days for this organization." />
		</div>
		<div class="grid-2 spaced">
			<Toast tone="success" title="Host enrolled">win-fs01 is online.</Toast>
			<Toast tone="danger" title="The job failed">restic exit code 12.</Toast>
		</div>
		<div class="demo-row spaced">
			<div class="demo-state">
				<span class="demo-label">tooltip</span>
				<Tooltip content="Copy snapshot ID" ariaLabel="Copy"><span>Copy</span></Tooltip>
			</div>
			<div class="demo-state">
				<span class="demo-label">popover</span>
				<Popover label="Retention details">
					{#snippet trigger()}Retention{/snippet}
					Keeps 7 daily and 4 weekly snapshots.
				</Popover>
			</div>
		</div>
	</section>

	<section>
		<h2>Menus, dialogs and side panel</h2>
		<div class="demo-row">
			<DropdownMenu items={menuItems}>Action menu</DropdownMenu>
			<DropdownMenu items={menuItems} icon="more" label="More" />
			<Button onclick={() => (confirmOpen = true)}>Open a confirm dialog</Button>
			<Button onclick={() => (destructiveOpen = true)}>Open a destructive dialog</Button>
			<Button onclick={() => (formOpen = true)}>Open a form dialog</Button>
			<Button onclick={() => (panelOpen = true)}>Open the side panel</Button>
		</div>
	</section>

	<section>
		<h2>Navigation</h2>
		<Tabs bind:value={tab} label="Views" items={tabs}>
			{#snippet children(value)}
				<p class="tab-panel">The {value} panel.</p>
			{/snippet}
		</Tabs>
		<div class="spaced">
			<Pagination bind:page pageCount={7} summary="Showing 1 to 20 of 128" />
		</div>
	</section>

	<section>
		<h2>ListView</h2>
		<ListView
			label="Jobs"
			items={runs}
			itemKey={(run) => run.id}
			icon={(run) => iconOf(jobState(run.job))}
			pill={(run) => pillOf(jobState(run.job))}
			title={(run) => run.name}
			href={() => '#'}
			meta={(run) => run.meta}
			aside={(run) => run.duration}
			groupBy={(run) => run.day}
			actions={(run) =>
				run.job.status === 'running'
					? [
							{
								id: 'cancel',
								label: 'Cancel',
								icon: 'circle-stop',
								danger: true,
								onSelect: () => {}
							}
						]
					: []}
		>
			{#snippet toolbar()}
				<FilterBar
					label="Job filters"
					value={listFilters}
					search={{ key: 'q', label: 'Search', placeholder: 'Search by plan, host, repository' }}
					states={{
						default: 'all',
						options: [
							{ value: 'all', label: 'All', count: 124 },
							{ value: 'running', label: 'Running', count: 2 },
							{ value: 'failed', label: 'Failed', count: 3, danger: true },
							{ value: 'succeeded', label: 'Succeeded', count: 118 }
						]
					}}
					facets={[
						{
							key: 'host',
							label: 'Host',
							options: [
								{ value: 'srv-db-01', count: 2 },
								{ value: 'MINDCORE', count: 1 }
							]
						}
					]}
					period
					sort={{
						default: 'started',
						options: [
							{ value: 'started', label: 'Started', order: 'desc' },
							{ value: 'duration', label: 'Duration', order: 'desc' }
						]
					}}
					onchange={(patch) => (listFilters = { ...listFilters, ...patch })}
				/>
			{/snippet}
		</ListView>
	</section>

	<section>
		<h2>DataTable</h2>
		<DataTable
			{columns}
			{rows}
			rowKey={(row) => row.id}
			sort={{ key: 'name', direction: 'asc' }}
			actions={(row) => [
				{ id: 'refresh', label: `Refresh ${row.name}`, icon: 'refresh', onSelect: () => {} },
				{
					id: 'remove',
					label: `Remove ${row.name}`,
					icon: 'trash',
					danger: true,
					onSelect: () => {}
				}
			]}
		>
			{#snippet toolbar()}
				<FilterBar
					label="Host filters"
					value={galleryFilters}
					search={{ key: 'q', label: 'Search' }}
					facets={[
						{
							key: 'status',
							label: 'Status',
							multiple: false,
							options: [
								{ value: 'online', label: 'Online', count: 2 },
								{ value: 'offline', label: 'Offline', count: 1 }
							]
						}
					]}
					period
					onchange={(patch) => (galleryFilters = { ...galleryFilters, ...patch })}
				/>
			{/snippet}
		</DataTable>
		<div class="grid-2 spaced">
			<DataTable {columns} rows={[]} rowKey={(row) => row.id} loading pageSize={3} />
			<Panel title="Empty state in a box" flush>
				<EmptyState
					icon="host"
					title="No hosts yet"
					description="Enroll the first agent to start a backup. The installation script holds a one-time token."
				>
					<Button variant="primary">Enroll a host</Button>
					<Button>Read the guide</Button>
				</EmptyState>
			</Panel>
		</div>
	</section>

	<section>
		<h2>Progress and feedback</h2>
		<div class="demo-cell">
			<span class="demo-label">Skeletons</span>
			<Skeleton variant="avatar" />
			<Skeleton variant="row" />
			<Skeleton />
		</div>
	</section>

	<section>
		<h2>Data views</h2>
		<div class="grid-2">
			<div class="demo-cell">
				<span class="demo-label">Key-value list</span>
				<KeyValue items={record} boxed />
			</div>
			<div class="demo-cell">
				<span class="demo-label">JSON diff</span>
				<JsonDiff {diff} variant="lines" />
				<JsonDiff {diff} />
			</div>
			<div class="demo-cell">
				<span class="demo-label">Copy field (secret shown once)</span>
				<Alert tone="attention" icon="key" title="Copy the token value">
					This value appears one time only.
				</Alert>
				<CopyField label="New token" value="rh_EXAMPLEONLYVALUE0000000000000000000000" secret />
			</div>
		</div>
	</section>
</div>

<ConfirmDialog
	bind:open={confirmOpen}
	title="Cancel the job?"
	message="The job j_4821 stops after the current step. The snapshot stays in the repository."
	cancelLabel="Keep running"
	confirmLabel="Cancel job"
	tone="danger"
	onConfirm={() => (confirmOpen = false)}
/>

<Dialog bind:open={destructiveOpen} title="Delete the repository?" icon="octagon" closable={false}>
	<p>
		This removes the configuration s3-prod-primary. The data in the bucket stays. Data loss can
		occur if the bucket is removed too.
	</p>
	{#snippet footer()}
		<Button onclick={() => (destructiveOpen = false)}>Cancel</Button>
		<Button variant="danger" onclick={() => (destructiveOpen = false)}>Delete</Button>
	{/snippet}
</Dialog>

<Dialog bind:open={formOpen} title="Create a user" onsubmit={() => (formOpen = false)}>
	<TextField id="gallery-username" label="User name" bind:value={username} required />
	<TextField id="gallery-display-name" label="Display name" bind:value={displayName} />
	<Select id="gallery-role" label="Role" bind:value={role} options={roles} />
	{#snippet footer()}
		<Button onclick={() => (formOpen = false)}>Cancel</Button>
		<Button variant="primary" type="submit">Create user</Button>
	{/snippet}
</Dialog>

<SidePanel bind:open={panelOpen} title="Snapshot a1b2c3d4">
	<KeyValue items={record} />
	{#snippet footer()}
		<Button variant="primary">Restore</Button>
		<Button onclick={() => (panelOpen = false)}>Close</Button>
	{/snippet}
</SidePanel>

<style>
	.gallery {
		max-width: 1440px;
		margin: 0 auto;
		padding: var(--rh-space-6) var(--rh-space-4) var(--rh-space-12);
	}

	section {
		margin-top: var(--rh-space-8);
	}

	h2,
	h3 {
		margin: 0 0 var(--rh-space-3);
		font-size: var(--rh-font-size-lg);
		font-weight: var(--rh-font-weight-semibold);
		line-height: var(--rh-line-height-tight);
	}

	h3 {
		margin-top: var(--rh-space-6);
	}

	.demo-row {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--rh-space-3);
		padding: var(--rh-space-4);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-canvas);
	}

	.icon-grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(112px, 1fr));
		gap: var(--rh-space-2);
	}

	.icon-cell {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: var(--rh-space-2);
		padding: var(--rh-space-3);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-canvas);
	}

	.demo-cell {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-2);
		padding: var(--rh-space-4);
		border: var(--rh-border-width) solid var(--rh-color-border);
		border-radius: var(--rh-radius-md);
		background-color: var(--rh-color-canvas);
	}

	.demo-state {
		display: flex;
		flex-direction: column-reverse;
		align-items: flex-start;
		gap: var(--rh-space-2);
	}

	.demo-label {
		color: var(--rh-color-text-subtle);
		font-size: var(--rh-font-size-xs);
		font-weight: var(--rh-font-weight-semibold);
	}

	.spaced {
		margin-top: var(--rh-space-4);
	}

	.stack {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-4);
	}

	.grid-2,
	.grid-3 {
		display: grid;
		gap: var(--rh-space-4);
		align-items: start;
	}

	.grid-2 {
		grid-template-columns: repeat(2, minmax(0, 1fr));
	}

	.grid-3 {
		grid-template-columns: repeat(3, minmax(0, 1fr));
	}

	.tab-panel {
		margin: 0;
		padding: var(--rh-space-4) 0;
	}

	.strong {
		font-weight: var(--rh-font-weight-semibold);
	}

	.sub {
		color: var(--rh-color-text-muted);
		font-size: var(--rh-font-size-sm);
	}
</style>
