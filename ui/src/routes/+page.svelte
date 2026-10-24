<script lang="ts">
	import AddLinksDialog, { extractLinks } from '#lib/components/AddLinksDialog.svelte';
	import DetailsPanel from '#lib/components/DetailsPanel.svelte';
	import DownloadsTable, { type Selection } from '#lib/components/DownloadsTable.svelte';
	import EmptyState from '#lib/components/EmptyState.svelte';
	import Icon from '#lib/components/Icon.svelte';
	import RemoveDialog from '#lib/components/RemoveDialog.svelte';
	import { downloads } from '#lib/downloads.svelte.js';

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

	// Ctrl+V anywhere on the page opens the dialog with the pasted links.
	function onpaste(e: ClipboardEvent) {
		if (adding || removing || inTextField(e.target)) return;
		const text = e.clipboardData?.getData('text') ?? '';
		if (extractLinks(text).length) {
			e.preventDefault();
			openAdd(text);
		}
	}
</script>

<svelte:window {onkeydown} {onpaste} />

<div class="page">
	<header class="toolbar">
		<h1>Downloads</h1>
		<span class="spacer"></span>
		<button class="btn accent" onclick={() => openAdd()} title="Add links (Ctrl+N, or paste anywhere)">
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
		<DownloadsTable packages={downloads.packages} live={downloads.live} bind:selection onactivate={activate} />
		<DetailsPanel {selection} onremove={askRemove} />
	{:else}
		<div class="fill">
			<EmptyState
				icon="downloads"
				title="No downloads yet"
				body="Click Add links, or just press Ctrl+V anywhere in this window to paste links."
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
