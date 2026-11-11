<script lang="ts">
	import Dialog from '#lib/components/Dialog.svelte';
	import EmptyState from '#lib/components/EmptyState.svelte';
	import Icon from '#lib/components/Icon.svelte';
	import { commands, isTauri, unwrap, type HistoryEntry } from '#lib/api.js';
	import { formatBytes, formatDate, iconFor } from '#lib/format.js';

	const PAGE = 200;

	let entries = $state<HistoryEntry[]>([]);
	let search = $state('');
	let limit = $state(PAGE);
	let ready = $state(false);
	let error = $state<string | null>(null);
	let clearing = $state(false);

	async function load(query: string, max: number) {
		try {
			entries = isTauri()
				? await unwrap(commands.historyList(query.trim() || null, max))
				: (await import('#lib/mock.js')).history(query);
			error = null;
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		} finally {
			ready = true;
		}
	}

	// Reload as the search changes, a moment after typing stops.
	$effect(() => {
		const query = search;
		const max = limit;
		const timer = setTimeout(() => load(query, max), query ? 200 : 0);
		return () => clearTimeout(timer);
	});

	async function run(action: () => Promise<unknown>) {
		try {
			await action();
			error = null;
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		}
	}

	const open = (e: HistoryEntry, reveal: boolean) => run(() => unwrap(commands.historyOpen(e.id, reveal)));

	async function forget(e: HistoryEntry) {
		await run(() => unwrap(commands.historyRemove([e.id])));
		entries = entries.filter((x) => x.id !== e.id);
	}

	async function clearAll() {
		clearing = false;
		await run(() => unwrap(commands.historyClear()));
		entries = [];
	}
</script>

<div class="page">
	<header class="toolbar">
		<h1>History</h1>
		<input
			class="textbox search"
			type="search"
			placeholder="Search names, links, folders"
			aria-label="Search history"
			bind:value={search}
		/>
		<span class="spacer"></span>
		<button class="btn" disabled={!entries.length || !isTauri()} onclick={() => (clearing = true)}>
			<Icon name="trash" /> Clear history
		</button>
	</header>

	{#if error}
		<p class="banner danger" role="alert">{error}</p>
	{/if}

	{#if !ready}
		<div class="fill"></div>
	{:else if !entries.length && !search}
		<div class="fill">
			<EmptyState
				icon="history"
				title="No history yet"
				body="Finished downloads are recorded here, and the LinkGrabber uses them to warn you before downloading something twice."
			/>
		</div>
	{:else}
		<div class="list" role="table" aria-label="Finished downloads">
			<div class="row head" role="row">
				<span role="columnheader">Name</span>
				<span role="columnheader">Package</span>
				<span role="columnheader" class="num">Size</span>
				<span role="columnheader">Finished</span>
				<span role="columnheader"><span class="sr-only">Actions</span></span>
			</div>
			{#each entries as e (e.id)}
				<div class="row" class:gone={!e.exists} role="row">
					<span class="name" role="cell" title={e.path}>
						<Icon name={iconFor(e.kind ?? 'http')} />
						<span class="label">
							<span class="title">{e.name}</span>
							<span class="meta">{e.exists ? e.url : `Moved or deleted · ${e.url}`}</span>
						</span>
					</span>
					<span class="muted ellipsis" role="cell">{e.package ?? '—'}</span>
					<span class="num" role="cell">{formatBytes(e.size)}</span>
					<span class="muted" role="cell">{formatDate(e.finishedAt)}</span>
					<span class="actions" role="cell">
						<button class="btn ghost icon-only" title="Open" aria-label="Open {e.name}" disabled={!e.exists} onclick={() => open(e, false)}>
							<Icon name="open" />
						</button>
						<button
							class="btn ghost icon-only"
							title="Show in folder"
							aria-label="Show {e.name} in folder"
							disabled={!e.exists}
							onclick={() => open(e, true)}
						>
							<Icon name="folder" />
						</button>
						<button class="btn ghost icon-only" title="Forget (keeps the file)" aria-label="Forget {e.name}" onclick={() => forget(e)}>
							<Icon name="close" />
						</button>
					</span>
				</div>
			{:else}
				<p class="none">Nothing matches “{search}”.</p>
			{/each}
			{#if entries.length >= limit}
				<button class="btn more" onclick={() => (limit += PAGE)}>Show more</button>
			{/if}
		</div>
	{/if}
</div>

<Dialog bind:open={clearing} title="Clear history?" width={420} onsubmit={clearAll}>
	<p class="confirm">
		All {entries.length >= limit ? `${limit}+` : entries.length} entries are forgotten, so earlier downloads no longer
		trigger duplicate warnings. Downloaded files are not touched.
	</p>
	{#snippet footer()}
		<button type="button" class="btn ghost" onclick={() => (clearing = false)}>Cancel</button>
		<button type="submit" class="btn danger">Clear history</button>
	{/snippet}
</Dialog>

<style>
	.page {
		display: flex;
		flex-direction: column;
		height: 100%;
	}
	.toolbar {
		display: flex;
		align-items: center;
		gap: var(--space-3);
		height: var(--toolbar-h);
		padding: 0 var(--space-5);
		background: var(--canvas);
		border-bottom: 1px solid var(--border-subtle);
	}
	h1 {
		margin: 0 var(--space-4) 0 0;
		font: var(--type-heading);
	}
	.search {
		width: 280px;
	}
	.spacer,
	.fill {
		flex: 1;
	}
	.banner {
		margin: var(--space-4) var(--page-pad) 0;
	}
	.list {
		flex: 1;
		overflow-y: auto;
		padding: 0 var(--space-2) var(--space-5);
	}
	.row {
		display: grid;
		grid-template-columns: minmax(240px, 1fr) minmax(100px, 200px) 88px 150px 104px;
		align-items: center;
		gap: var(--space-4);
		min-height: var(--row-h);
		padding: var(--space-2) var(--space-3);
		font: var(--type-body);
		border-bottom: 1px solid var(--border-subtle);
	}
	.row:not(.head):hover {
		background: var(--hover);
	}
	.head {
		position: sticky;
		top: 0;
		z-index: 1;
		height: var(--control-h);
		min-height: var(--control-h);
		padding: 0 var(--space-3);
		font: var(--type-caption);
		color: var(--text-muted);
		background: var(--canvas);
		border-bottom: 1px solid var(--border-subtle);
	}
	.head:hover {
		background: var(--canvas);
	}
	.gone .title,
	.gone .name {
		color: var(--text-secondary);
	}
	.name {
		display: flex;
		align-items: center;
		gap: var(--space-3);
		min-width: 0;
	}
	.label {
		display: flex;
		flex-direction: column;
		min-width: 0;
	}
	.title,
	.meta,
	.ellipsis {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.meta {
		font: var(--type-small);
		color: var(--text-muted);
	}
	.num {
		text-align: right;
		font-variant-numeric: tabular-nums;
	}
	.actions {
		display: flex;
		justify-content: flex-end;
		gap: var(--space-1);
	}
	.none {
		margin: 0;
		padding: var(--space-5) var(--space-3);
		font: var(--type-body);
		color: var(--text-secondary);
	}
	.more {
		margin: var(--space-4) auto 0;
		display: block;
	}
	.confirm {
		margin: 0;
		font: var(--type-body);
		color: var(--text-secondary);
	}
	.sr-only {
		position: absolute;
		width: 1px;
		height: 1px;
		overflow: hidden;
		clip: rect(0 0 0 0);
	}
</style>
