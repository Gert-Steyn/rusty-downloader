<script lang="ts">
	import AddLinksDialog from '#lib/components/AddLinksDialog.svelte';
	import ContextMenu, { type MenuItem } from '#lib/components/ContextMenu.svelte';
	import DetailsPanel from '#lib/components/DetailsPanel.svelte';
	import DownloadsTable, { type Selection } from '#lib/components/DownloadsTable.svelte';
	import EmptyState from '#lib/components/EmptyState.svelte';
	import Icon from '#lib/components/Icon.svelte';
	import RemoveDialog from '#lib/components/RemoveDialog.svelte';
	import SpeedLimitDialog from '#lib/components/SpeedLimitDialog.svelte';
	import { downloads } from '#lib/downloads.svelte.js';
	import { formatSpeed } from '#lib/format.js';

	let selection = $state<Selection>(null);
	let adding = $state(false);
	let pastedText = $state('');
	let removing = $state(false);
	let removeTarget = $state<Selection>(null);

	const hasAny = $derived(downloads.packages.length > 0);

	// Forget the selection once its row is gone.
	$effect(() => {
		if (!selection) return;
		const exists =
			selection.kind === 'download'
				? downloads.download(selection.id) !== undefined
				: downloads.package(selection.id) !== undefined;
		if (!exists) selection = null;
	});

	function openAdd(text = '') {
		pastedText = text;
		adding = true;
	}

	function askRemove() {
		if (!selection) return;
		removeTarget = selection;
		removing = true;
	}

	function activate(target: NonNullable<Selection>) {
		if (target.kind !== 'download') return;
		const d = downloads.download(target.id);
		if (d?.status === 'completed') downloads.open(d.id);
	}

	function togglePause(target: NonNullable<Selection>) {
		const items =
			target.kind === 'download'
				? [downloads.download(target.id)]
				: (downloads.package(target.id)?.downloads ?? []);
		const running = items.some((d) => d?.status === 'active' || d?.status === 'queued');
		for (const d of items) {
			if (!d) continue;
			if (running) downloads.pause(d.id);
			else if (d.status !== 'completed') downloads.resume(d.id);
		}
	}

	// Right-click menu for the row under the pointer.
	let menuAt = $state<{ x: number; y: number } | null>(null);
	let menuItems = $state<MenuItem[]>([]);
	let limiting = $state<number | null>(null);
	let limitOpen = $state(false);

	function openMenu(target: NonNullable<Selection>, x: number, y: number) {
		menuItems = target.kind === 'download' ? downloadMenu(target.id) : packageMenu(target.id);
		menuAt = { x, y };
	}

	function downloadMenu(id: number): MenuItem[] {
		const d = downloads.download(id);
		if (!d) return [];
		const running = d.status === 'active' || d.status === 'queued';
		return [
			running
				? { label: 'Pause', icon: 'pause', action: () => downloads.pause(id) }
				: {
						label: d.status === 'paused' ? 'Resume' : 'Retry',
						icon: 'play',
						disabled: d.status === 'completed',
						action: () => downloads.resume(id)
					},
			{ label: 'Open', icon: 'open', disabled: d.status !== 'completed', action: () => downloads.open(id) },
			{ label: 'Show in folder', icon: 'folder', action: () => downloads.reveal(id) },
			{ label: 'Copy link', icon: 'copy', action: () => copy(d.sourceUrl) },
			{
				label: d.speedLimit ? `Speed limit (${formatSpeed(d.speedLimit)})…` : 'Speed limit…',
				icon: 'speed',
				disabled: d.status === 'completed',
				action: () => {
					limiting = id;
					limitOpen = true;
				}
			},
			'separator',
			{ label: 'Remove…', icon: 'trash', danger: true, action: askRemove }
		];
	}

	function packageMenu(id: number): MenuItem[] {
		const p = downloads.package(id);
		if (!p) return [];
		const ids = downloads.packages.map((q) => q.id);
		const at = ids.indexOf(id);
		const running = p.downloads.some((d) => d.status === 'active' || d.status === 'queued');
		const resumable = p.downloads.some((d) => d.status !== 'completed');
		return [
			running
				? { label: 'Pause all', icon: 'pause', action: () => p.downloads.forEach((d) => downloads.pause(d.id)) }
				: {
						label: 'Resume all',
						icon: 'play',
						disabled: !resumable,
						action: () => p.downloads.forEach((d) => d.status !== 'completed' && downloads.resume(d.id))
					},
			{ label: 'Open folder', icon: 'folder', action: () => downloads.openFolder(id) },
			{ label: 'Copy links', icon: 'copy', action: () => copy(p.downloads.map((d) => d.sourceUrl).join('\n')) },
			'separator',
			{ label: 'Move to top', icon: 'move-top', disabled: at <= 0, action: () => downloads.movePackage(id, ids[0]) },
			{
				label: 'Move to bottom',
				icon: 'move-bottom',
				disabled: at === ids.length - 1,
				action: () => downloads.movePackage(id, null)
			},
			'separator',
			{ label: 'Remove…', icon: 'trash', danger: true, action: askRemove }
		];
	}

	async function copy(text: string) {
		try {
			await navigator.clipboard.writeText(text);
		} catch (e) {
			downloads.error = `Could not copy: ${e}`;
		}
	}

	function inTextField(target: EventTarget | null): boolean {
		return target instanceof HTMLElement && target.closest('input, textarea, [contenteditable]') !== null;
	}

	function onkeydown(e: KeyboardEvent) {
		if (adding || removing || inTextField(e.target)) return;
		if ((e.ctrlKey && e.key.toLowerCase() === 'n') || e.key === 'Insert') {
			openAdd();
		} else if (e.key === 'Delete' && selection) {
			askRemove();
		} else if (e.key === ' ' && selection) {
			togglePause(selection);
		} else {
			return;
		}
		e.preventDefault();
	}

</script>

<svelte:window {onkeydown} />

<div class="page">
	<header class="toolbar">
		<h1>Downloads</h1>
		<span class="spacer"></span>
		<button class="btn accent" onclick={() => openAdd()} title="Add and start links (Ctrl+N)">
			<Icon name="plus" /> Add links
		</button>
		<button class="btn" onclick={downloads.resumeAll} disabled={!hasAny}><Icon name="play" /> Resume all</button>
		<button class="btn" onclick={downloads.pauseAll} disabled={!hasAny}><Icon name="pause" /> Pause all</button>
	</header>

	{#if downloads.error}
		<div class="banner" role="alert">
			<span>{downloads.error}</span>
			<button class="btn" onclick={() => (downloads.error = null)} aria-label="Dismiss">
				<Icon name="close" />
			</button>
		</div>
	{/if}

	{#if !downloads.ready}
		<div class="fill"></div>
	{:else if hasAny}
		<DownloadsTable
			packages={downloads.packages}
			live={downloads.live}
			bind:selection
			onactivate={activate}
			onmenu={openMenu}
			onmovepackage={downloads.movePackage}
		/>
		<DetailsPanel {selection} onremove={askRemove} />
	{:else}
		<div class="fill">
			<EmptyState
				icon="downloads"
				title="No downloads yet"
				body="Click Add links to start right away, or paste (Ctrl+V) or drop links anywhere to review them in the LinkGrabber first."
			/>
		</div>
	{/if}
</div>

<AddLinksDialog
	bind:open={adding}
	initialText={pastedText}
	onadded={(id) => (selection = { kind: 'package', id })}
/>
<RemoveDialog bind:open={removing} target={removeTarget} />
<SpeedLimitDialog bind:open={limitOpen} id={limiting} />
<ContextMenu bind:at={menuAt} items={menuItems} />


<style>
	.page {
		display: flex;
		flex-direction: column;
		height: 100%;
	}
	.toolbar {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 16px 20px 12px;
	}
	h1 {
		margin: 0;
		font: 600 20px/28px var(--font-display);
	}
	.spacer,
	.fill {
		flex: 1;
	}
	.banner {
		display: flex;
		align-items: center;
		gap: 12px;
		margin: 0 20px 12px;
		padding: 6px 6px 6px 16px;
		border-radius: var(--radius-control);
		border: 1px solid var(--stroke-card);
		background: color-mix(in srgb, var(--critical) 12%, transparent);
	}
	.banner span {
		flex: 1;
		overflow-wrap: anywhere;
	}
</style>
