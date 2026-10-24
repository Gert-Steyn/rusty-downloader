<script lang="ts">
	import { downloads } from '#lib/downloads.svelte.js';
	import { displayName } from '#lib/format.js';
	import type { Selection } from './DownloadsTable.svelte';
	import Dialog from './Dialog.svelte';

	let {
		open = $bindable(false),
		target,
		onremoved
	}: { open?: boolean; target: Selection; onremoved?: () => void } = $props();

	let deleteFiles = $state(false);

	const subject = $derived.by(() => {
		if (target?.kind === 'download') {
			const d = downloads.download(target.id);
			return d ? { name: displayName(d), completed: d.status === 'completed' ? 1 : 0 } : null;
		}
		if (target?.kind === 'package') {
			const p = downloads.package(target.id);
			return p ? { name: p.name, completed: p.downloads.filter((d) => d.status === 'completed').length } : null;
		}
		return null;
	});

	$effect(() => {
		if (open) deleteFiles = false;
	});

	async function confirm() {
		if (!target) return;
		open = false;
		if (target.kind === 'download') await downloads.remove(target.id, deleteFiles);
		else await downloads.removePackage(target.id, deleteFiles);
		onremoved?.();
	}
</script>

<Dialog bind:open title={target?.kind === 'package' ? 'Remove package?' : 'Remove download?'} width={440} onsubmit={confirm}>
	<p>
		<strong>{subject?.name}</strong> will be removed from the list. Unfinished parts are deleted.
	</p>
	{#if subject?.completed}
		<label class="check">
			<input type="checkbox" bind:checked={deleteFiles} />
			Also delete {subject.completed === 1 ? 'the downloaded file' : `${subject.completed} downloaded files`} from disk
		</label>
	{/if}

	{#snippet footer()}
		<button type="submit" class="btn accent">Remove</button>
		<button type="button" class="btn" onclick={() => (open = false)}>Cancel</button>
	{/snippet}
</Dialog>

<style>
	p {
		margin: 0 0 16px;
		overflow-wrap: anywhere;
	}
	.check {
		display: flex;
		align-items: center;
		gap: 10px;
	}
</style>
