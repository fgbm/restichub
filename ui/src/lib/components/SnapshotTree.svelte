<script lang="ts">
	import { resolve } from '$app/paths';
	import { untrack } from 'svelte';
	import { ApiError, service } from '#lib/api/index.ts';
	import type { NodeItem, SnapshotItem } from '#lib/api/index.ts';
	import { Button, Icon } from '#lib/components/ui/index.ts';
	import { formatBytes } from '#lib/format.ts';
	import { t, tErrorCode, tOr } from '#lib/i18n/index.ts';
	import { iconOfNode, isCovered } from '#lib/restore.ts';
	import { BrowseTimeout, settle } from '#lib/restore-poll.ts';

	/** The page size that the screen asks for. The agent returns at most this many entries. */
	const PAGE = 1000;

	interface Directory {
		nodes: NodeItem[];
		/** The offset of the next page, or null when the directory is complete. */
		next: number | null;
		status: 'loading' | 'idle' | 'failed';
		error: string | null;
		/** The job whose log tells why the directory failed. */
		jobId?: string | null;
		expanded: boolean;
	}

	/** A browse job that ended without a listing. */
	class BrowseFailed extends Error {
		code: string;
		jobId: string | null;
		constructor(code: string, jobId: string | null) {
			super(code);
			this.code = code;
			this.jobId = jobId;
		}
	}

	let {
		repositoryId,
		snapshot,
		agentId = null,
		selected = $bindable(new Set<string>())
	}: {
		repositoryId: string;
		snapshot: SnapshotItem;
		/** The host that reads the repository. Null uses the maintenance host. */
		agentId?: string | null;
		/** The paths that the restore includes. A directory holds its whole content. */
		selected?: Set<string>;
	} = $props();

	let directories = $state<Record<string, Directory>>({});
	// A new snapshot invalidates the answers of the old one that are still on their way.
	let generation = 0;

	function describe(problem: unknown): string {
		if (problem instanceof ApiError) return tErrorCode(problem.code) ?? problem.title;
		if (problem instanceof BrowseTimeout) return problem.message;
		if (problem instanceof BrowseFailed) return tOr(`jobs.reason.${problem.code}`, problem.code);
		return t('table.error');
	}

	async function load(path: string): Promise<void> {
		const mine = generation;
		const current = directories[path];
		const offset = current?.next ?? 0;
		directories[path] = {
			nodes: current?.nodes ?? [],
			next: current?.next ?? null,
			status: 'loading',
			error: null,
			expanded: true
		};
		try {
			const request = {
				snapshot_id: snapshot.id,
				path,
				offset,
				limit: PAGE,
				agent_id: agentId
			};
			const first = await service.startLs(repositoryId, request);
			const view = await settle(first, (item) => service.getLs(repositoryId, item.job_id), {
				cancelled: () => mine !== generation
			});
			if (!view || mine !== generation) return;
			if (view.state === 'failed') throw new BrowseFailed(view.error_code ?? 'failed', view.job_id);
			const known = directories[path];
			directories[path] = {
				nodes: [...(offset === 0 ? [] : (known?.nodes ?? [])), ...view.nodes],
				next: view.last ? null : (view.next_offset ?? null),
				status: 'idle',
				error: null,
				expanded: true
			};
		} catch (problem) {
			if (mine !== generation) return;
			const known = directories[path];
			directories[path] = {
				nodes: known?.nodes ?? [],
				next: known?.next ?? null,
				status: 'failed',
				error: describe(problem),
				jobId: problem instanceof BrowseFailed ? problem.jobId : null,
				expanded: true
			};
		}
	}

	$effect(() => {
		// The effect depends on the snapshot and the repository only; the load reads and writes the
		// state of the directories.
		void snapshot.id;
		void repositoryId;
		untrack(() => {
			generation += 1;
			directories = {};
			void load('');
		});
	});

	function toggle(path: string): void {
		const known = directories[path];
		if (!known) {
			void load(path);
		} else {
			directories[path] = { ...known, expanded: !known.expanded };
		}
	}

	function choose(path: string, checked: boolean): void {
		const others = [...selected].filter((item) => item !== path);
		selected = new Set(checked ? [...others, path] : others);
	}
</script>

{#snippet branch(path: string)}
	{@const directory = directories[path]}
	{#if directory}
		<ul
			class="tree__list"
			role={path === '' ? 'tree' : 'group'}
			aria-label={path === '' ? t('restore.treeLabel') : undefined}
		>
			{#each directory.nodes as node (node.path)}
				{@const child = directories[node.path]}
				{@const covered = isCovered(node.path, selected)}
				<li
					class="tree__item"
					role="treeitem"
					aria-selected={selected.has(node.path) || covered}
					aria-expanded={node.kind === 'dir' ? !!child?.expanded : undefined}
				>
					<div class="tree__row">
						{#if node.kind === 'dir'}
							<button
								type="button"
								class="tree__caret"
								aria-label={child?.expanded
									? t('restore.collapse', { name: node.name })
									: t('restore.expand', { name: node.name })}
								onclick={() => toggle(node.path)}
							>
								<Icon name={child?.expanded ? 'chevron-down' : 'chevron-right'} size="xs" />
							</button>
						{:else}
							<span class="tree__caret tree__caret--none"></span>
						{/if}
						<input
							type="checkbox"
							class="tree__check"
							checked={selected.has(node.path) || covered}
							disabled={covered}
							aria-label={t('restore.select', { name: node.name })}
							onchange={(event) => choose(node.path, event.currentTarget.checked)}
						/>
						<span class="tree__icon tree__icon--{node.kind}">
							<Icon name={iconOfNode(node, !!child?.expanded)} />
						</span>
						<span class="tree__name" title={node.name}>{node.name}</span>
						{#if node.kind === 'symlink' && node.link_target}
							<span class="tree__target" title={node.link_target}>→ {node.link_target}</span>
						{:else if node.kind === 'file'}
							<span class="tree__meta">{formatBytes(node.size)}</span>
						{/if}
					</div>
					{#if node.kind === 'dir' && child?.expanded}
						{@render branch(node.path)}
					{/if}
				</li>
			{/each}
			{#if directory.status === 'loading'}
				<li class="tree__note" role="none">{t('restore.loading')}</li>
			{:else if directory.status === 'failed'}
				<li class="tree__note tree__note--error" role="none">
					<span>{t('restore.treeFailed', { reason: directory.error ?? '' })}</span>
					{#if directory.jobId}
						<a href={resolve('/(app)/jobs/[id]', { id: directory.jobId })}>{t('restore.openJob')}</a
						>
					{/if}
					<Button size="sm" onclick={() => load(path)}>{t('restore.retry')}</Button>
				</li>
			{:else if directory.nodes.length === 0}
				<li class="tree__note" role="none">{t('restore.emptyDirectory')}</li>
			{:else if directory.next !== null}
				<li class="tree__note" role="none">
					<Button size="sm" onclick={() => load(path)}>{t('restore.loadMore')}</Button>
				</li>
			{/if}
		</ul>
	{/if}
{/snippet}

<div class="tree">
	{@render branch('')}
</div>

<style>
	.tree {
		height: 100%;
		overflow: auto;
		font-size: var(--rh-font-size-sm);
	}

	.tree__list {
		margin: 0;
		padding: 0;
		list-style: none;
	}

	.tree__list .tree__list {
		padding-inline-start: var(--rh-space-5);
	}

	.tree__row {
		display: flex;
		align-items: center;
		gap: var(--rh-space-2);
		min-height: 2rem;
		min-width: 0;
		padding-inline: var(--rh-space-1);
		border-radius: var(--rh-radius-sm);
	}

	.tree__row:hover {
		background-color: var(--rh-color-hover);
	}

	.tree__caret {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 1.25rem;
		height: 1.25rem;
		padding: 0;
		border: 0;
		border-radius: var(--rh-radius-sm);
		background: none;
		color: var(--rh-color-text-muted);
		cursor: pointer;
	}

	.tree__caret--none {
		cursor: default;
	}

	.tree__check {
		flex: none;
		width: 1rem;
		height: 1rem;
		margin: 0;
		accent-color: var(--rh-color-accent);
	}

	.tree__icon {
		display: inline-flex;
		flex: none;
		color: var(--rh-color-text-muted);
	}

	.tree__icon--dir {
		color: var(--rh-color-link);
	}

	.tree__target {
		flex: 0 100 auto;
		min-width: 0;
		overflow: hidden;
		color: var(--rh-color-text-muted);
		font-family: var(--rh-font-mono, monospace);
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.tree__name {
		flex: 0 1 auto;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.tree__meta {
		flex: none;
		margin-inline-start: auto;
		color: var(--rh-color-text-muted);
		font-variant-numeric: tabular-nums;
		text-align: end;
		white-space: nowrap;
	}

	.tree__note {
		display: flex;
		align-items: center;
		gap: var(--rh-space-3);
		padding: var(--rh-space-2) var(--rh-space-2) var(--rh-space-2) var(--rh-space-6);
		color: var(--rh-color-text-muted);
	}

	.tree__note--error {
		color: var(--rh-color-danger-text);
	}
</style>
