<script lang="ts">
	import type { Download } from '#lib/api.js';
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

	/** Files on disk that "delete" would remove: a gallery's finished files, even if it stopped part-way. */
	function filesOnDisk(d: Download): number {
		if (d.files) return d.files.done;
		return d.status === 'completed' ? 1 : 0;
	}

	const subject = $derived.by(() => {
		if (target?.kind === 'download') {
			const d = downloads.download(target.id);
			return d ? { name: displayName(d), completed: filesOnDisk(d) } : null;
		}
		if (target?.kind === 'package') {
			const p = downloads.package(target.id);
			return p ? { name: p.name, completed: p.downloads.reduce((n, d) => n + filesOnDisk(d), 0) } : null;
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

<Dialog
	bind:open
	title={subject
		? `Remove '${subject.name}'?`
		: target?.kind === 'package'
			? 'Remove package?'
			: 'Remove download?'}
	width={440}
	onsubmit={confirm}
>
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
		<button type="button" class="btn ghost" onclick={() => (open = false)}>Cancel</button>
		<button type="submit" class="btn danger">Remove</button>
	{/snippet}
</Dialog>

<style>
	p {
		margin: 0 0 var(--space-5);
		overflow-wrap: anywhere;
		font: var(--type-body);
		color: var(--text-secondary);
	}
	strong {
		font: var(--type-label);
		color: var(--text);
	}
	.check {
		display: flex;
		align-items: center;
		gap: var(--space-3);
		font: var(--type-body);
		color: var(--text);
	}
</style>
