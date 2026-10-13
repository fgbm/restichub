<script lang="ts">
	/*
	 * Component gallery: every element of `ui/` in its states.
	 *
	 * This is a development tool, so the labels are English and it is not part of a
	 * production screen. The route `(dev)/components` mounts it in a dev build only.
	 */
	import {
		Alert,
		Badge,
		Button,
		Card,
		Checkbox,
		ConfirmDialog,
		Dialog,
		DropdownMenu,
		EmptyState,
		PageHeader,
		Pagination,
		Panel,
		PasswordField,
		Select,
		Skeleton,
		StatusPill,
		Switch,
		Table,
		Tabs,
		TextField,
		Toast,
		Tooltip
	} from '#lib/components/ui/index.ts';
	import type { MenuItem, TableColumn } from '#lib/components/ui/index.ts';

	let dialogOpen = $state(false);
	let confirmOpen = $state(false);
	let page = $state(2);
	let tab = $state('general');

	const menuItems: MenuItem[] = [
		{ label: 'Edit' },
		{ label: 'Duplicate' },
		{ label: 'Delete', tone: 'danger', separatorBefore: true }
	];

	const tabs = [
		{ value: 'general', label: 'General', count: 3 },
		{ value: 'access', label: 'Access' }
	];

	interface Host {
		id: string;
		name: string;
		status: string;
		size: string;
	}

	const columns: TableColumn<Host>[] = [
		{ key: 'name', header: 'Host' },
		{ key: 'status', header: 'Status' },
		{ key: 'size', header: 'Size', align: 'right' }
	];
	const rows: Host[] = [
		{ id: 'h1', name: 'host-01', status: 'online', size: '12 GB' },
		{ id: 'h2', name: 'host-02', status: 'offline', size: '8 GB' }
	];
</script>

<div class="gallery">
	<PageHeader title="Components" description="Every element in its states." bordered />

	<section>
		<h2>Buttons</h2>
		<div class="row">
			<Button variant="primary">Primary</Button>
			<Button>Secondary</Button>
			<Button variant="danger">Danger</Button>
			<Button variant="ghost">Ghost</Button>
			<Button size="sm">Small</Button>
			<Button loading>Loading</Button>
			<Button disabled>Disabled</Button>
		</div>
	</section>

	<section>
		<h2>Fields</h2>
		<div class="grid">
			<TextField label="Name" value="host-01" help="Lower case only" />
			<TextField label="Name" error="Required" />
			<PasswordField label="Password" value="secret" />
			<Select label="Role" options={[{ value: 'admin', label: 'Admin' }]} value="admin" />
			<Checkbox label="Enabled" checked />
			<Switch label="Active" checked />
		</div>
	</section>

	<section>
		<h2>Badges</h2>
		<div class="row">
			<Badge>Neutral</Badge>
			<Badge tone="accent">Accent</Badge>
			<Badge tone="success">Success</Badge>
			<Badge tone="warning">Warning</Badge>
			<Badge tone="danger">Danger</Badge>
			<StatusPill tone="success">Success</StatusPill>
			<StatusPill tone="running">Running</StatusPill>
			<StatusPill tone="queued">Queued</StatusPill>
			<StatusPill tone="failed">Failed</StatusPill>
			<StatusPill tone="cancelled">Cancelled</StatusPill>
		</div>
	</section>

	<section>
		<h2>Alerts and toasts</h2>
		<div class="stack">
			<Alert tone="info" title="Information">A neutral note.</Alert>
			<Alert tone="success" title="Saved">The tenant is ready.</Alert>
			<Alert tone="attention" title="Attention">Check the key.</Alert>
			<Alert tone="danger" title="Failed">The save did not complete.</Alert>
		</div>
		<div class="stack">
			<Toast tone="success" title="Saved">The tenant is ready.</Toast>
			<Toast tone="danger" title="Failed">The save did not complete.</Toast>
		</div>
	</section>

	<section>
		<h2>Overlays</h2>
		<div class="row">
			<Button onclick={() => (dialogOpen = true)}>Open dialog</Button>
			<Button variant="danger" onclick={() => (confirmOpen = true)}>Open confirm</Button>
			<DropdownMenu items={menuItems}><span>Menu</span></DropdownMenu>
			<Tooltip content="Copy the value"><span>Tooltip</span></Tooltip>
		</div>
	</section>

	<section>
		<h2>Tabs</h2>
		<Tabs bind:value={tab} label="Views" items={tabs}>
			{#snippet children(value)}
				<p>Panel {value}</p>
			{/snippet}
		</Tabs>
	</section>

	<section>
		<h2>Table</h2>
		<Table {columns} {rows} rowKey={(row) => row.id} caption="Hosts">
			{#snippet rowActions(row)}
				<Button variant="ghost" size="sm">Open {row.name}</Button>
			{/snippet}
		</Table>
	</section>

	<section>
		<h2>Cards, panels, states</h2>
		<div class="grid">
			<Card><p>Card content</p></Card>
			<Panel title="Panel" subtitle="A titled box"><p>Panel content</p></Panel>
			<EmptyState title="No hosts" description="Add the first host.">
				<Button variant="primary">Add host</Button>
			</EmptyState>
		</div>
		<div class="stack">
			<Skeleton />
			<Skeleton variant="row" />
			<Skeleton variant="avatar" />
		</div>
	</section>

	<section>
		<h2>Pagination</h2>
		<Pagination bind:page pageCount={5} summary="Showing 11 to 20 of 45" />
	</section>
</div>

<Dialog bind:open={dialogOpen} title="Dialog" description="A description.">
	<p>Dialog body.</p>
	{#snippet footer()}
		<Button onclick={() => (dialogOpen = false)}>Close</Button>
	{/snippet}
</Dialog>

<ConfirmDialog
	bind:open={confirmOpen}
	title="Delete host"
	message="This cannot be undone."
	confirmLabel="Delete"
	tone="danger"
/>

<style>
	.gallery {
		max-width: var(--rh-layout-content-max-width);
		margin: 0 auto;
		padding: var(--rh-space-6) var(--rh-space-4) var(--rh-space-12);
	}

	section {
		margin-top: var(--rh-space-8);
	}

	h2 {
		margin-bottom: var(--rh-space-3);
		font-size: var(--rh-font-size-lg);
		font-weight: var(--rh-font-weight-semibold);
	}

	.row {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--rh-space-2);
	}

	.stack {
		display: flex;
		flex-direction: column;
		gap: var(--rh-space-3);
		margin-top: var(--rh-space-3);
	}

	.grid {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
		gap: var(--rh-space-4);
	}
</style>
