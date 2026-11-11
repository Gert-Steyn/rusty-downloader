<script lang="ts">
	// A download's expected checksum: typed or pasted, checked when the file is there.
	import type { ChecksumAlgo } from '#lib/api.js';
	import { downloads } from '#lib/downloads.svelte.js';
	import { CHECKSUM_LABELS, displayName } from '#lib/format.js';
	import Dialog from './Dialog.svelte';

	let { open = $bindable(false), id }: { open?: boolean; id: number | null } = $props();

	const download = $derived(id == null ? undefined : downloads.download(id));
	let algo = $state<ChecksumAlgo | ''>('');
	let text = $state('');
	let error = $state<string | null>(null);

	$effect(() => {
		if (!open) return;
		const checksum = download?.checksum;
		algo = checksum?.algo ?? '';
		text = checksum?.value ?? '';
		error = null;
	});

	async function save(value: string) {
		if (id == null) return;
		try {
			await downloads.setChecksum(id, algo || null, value);
			open = false;
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		}
	}
</script>

<Dialog bind:open title="Checksum" width={520} onsubmit={() => save(text)}>
	<p>{download ? displayName(download) : ''}</p>
	<div class="entry">
		<select class="select" aria-label="Algorithm" bind:value={algo}>
			<option value="">Recognise it</option>
			{#each Object.entries(CHECKSUM_LABELS) as [value, label] (value)}
				<option {value}>{label}</option>
			{/each}
		</select>
		<input
			class="textbox mono"
			type="text"
			aria-label="Checksum"
			placeholder="Paste the checksum"
			spellcheck="false"
			aria-invalid={error != null}
			bind:value={text}
		/>
	</div>
	<small>
		"Recognise it" goes by length: 8 hex digits are CRC32, 32 MD5, 40 SHA-1, 64 SHA-256. A whole line like
		<span class="mono">SHA256: …</span> works too. A finished file is checked at once; a mismatch marks the download
		failed and keeps the file.
	</small>
	{#if error}<p class="error-text">{error}</p>{/if}

	{#snippet footer()}
		{#if download?.checksum}
			<button type="button" class="btn" onclick={() => save('')}>Remove</button>
		{/if}
		<button type="button" class="btn ghost" onclick={() => (open = false)}>Cancel</button>
		<button type="submit" class="btn accent">Save</button>
	{/snippet}
</Dialog>

<style>
	p {
		margin: 0 0 var(--space-4);
		overflow-wrap: anywhere;
		font: var(--type-body);
		color: var(--text-secondary);
	}
	.entry {
		display: flex;
		align-items: center;
		gap: var(--space-3);
		margin-bottom: var(--space-3);
	}
	.entry input {
		flex: 1;
		min-width: 0;
	}
	small {
		display: block;
		font: var(--type-small);
		color: var(--text-muted);
	}
	.error-text {
		margin: var(--space-3) 0 0;
	}
</style>
