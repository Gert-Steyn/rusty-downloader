<script lang="ts">
	// Choose which files of a torrent download. Everything selected saves as
	// "all of them" (null), so files added later are not an issue.
	import { untrack } from 'svelte';
	import type { TorrentInfo } from '#lib/api.js';
	import { formatBytes } from '#lib/format.js';
	import Dialog from './Dialog.svelte';

	let {
		open = $bindable(false),
		torrent,
		selected,
		onsave
	}: {
		open?: boolean;
		torrent: TorrentInfo;
		/** The chosen file indices, or `null` for all of them. */
		selected: number[] | null;
		onsave: (onlyFiles: number[] | null) => void;
	} = $props();

	let picked = $state<number[]>([]);

	$effect(() => {
		if (!open) return;
		untrack(() => {
			picked = selected ? [...selected] : torrent.files.map((f) => f.index);
		});
	});

	const total = $derived(torrent.files.length);
	const chosen = $derived(torrent.files.filter((f) => picked.includes(f.index)));
	const chosenSize = $derived(chosen.reduce((sum, f) => sum + f.size, 0));

	function toggle(index: number, on: boolean) {
		picked = on ? [...picked.filter((i) => i !== index), index] : picked.filter((i) => i !== index);
	}

	function save() {
		if (!picked.length) return;
		const all = chosen.length === total;
		open = false;
		onsave(all ? null : [...picked].sort((a, b) => a - b));
	}
</script>

<Dialog bind:open title="Files in {torrent.name}" width={560} onsubmit={save}>
	<div class="tools">
		<button type="button" class="btn ghost small" onclick={() => (picked = torrent.files.map((f) => f.index))}>
			Select all
		</button>
		<button type="button" class="btn ghost small" onclick={() => (picked = [])}>Select none</button>
	</div>
	<div class="scroll">
		<ul>
			{#each torrent.files as file (file.index)}
				<li>
					<label class="file" class:off={!picked.includes(file.index)}>
						<input
							type="checkbox"
							checked={picked.includes(file.index)}
							onchange={(e) => toggle(file.index, e.currentTarget.checked)}
						/>
						<span class="path">{file.path}</span>
						<span class="size num">{formatBytes(file.size)}</span>
					</label>
				</li>
			{/each}
		</ul>
	</div>
	<p class="summary num">{chosen.length} of {total} files · {formatBytes(chosenSize)}</p>

	{#snippet footer()}
		{#if !picked.length}
			<span class="hint">Choose at least one file</span>
		{/if}
		<button type="button" class="btn ghost" onclick={() => (open = false)}>Cancel</button>
		<button type="submit" class="btn accent" disabled={!picked.length}>Save</button>
	{/snippet}
</Dialog>

<style>
	.tools {
		display: flex;
		gap: var(--space-2);
		margin-bottom: var(--space-3);
	}
	.scroll {
		max-height: 50vh;
		overflow-y: auto;
		border: 1px solid var(--border-subtle);
		border-radius: var(--radius-md);
	}
	ul {
		margin: 0;
		padding: var(--space-1);
		list-style: none;
	}
	.file {
		display: grid;
		grid-template-columns: auto minmax(0, 1fr) auto;
		align-items: center;
		gap: var(--space-3);
		padding: var(--space-2) var(--space-3);
		border-radius: var(--radius-md);
		font: var(--type-body);
		color: var(--text);
	}
	.file:hover {
		background: var(--hover);
	}
	.file.off {
		color: var(--text-muted);
	}
	.path {
		overflow-wrap: anywhere;
	}
	.size {
		font: var(--type-small);
		color: var(--text-muted);
		text-align: right;
		white-space: nowrap;
	}
	.summary {
		margin: var(--space-3) 0 0;
		font: var(--type-small);
		color: var(--text-muted);
	}
	.hint {
		margin-right: auto;
		align-self: center;
	}
</style>
