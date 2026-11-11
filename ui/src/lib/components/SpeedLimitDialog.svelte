<script lang="ts">
	// A download's own speed limit, on top of the global one.
	import { downloads } from '#lib/downloads.svelte.js';
	import { displayName } from '#lib/format.js';
	import Dialog from './Dialog.svelte';

	let { open = $bindable(false), id }: { open?: boolean; id: number | null } = $props();

	const MB = 1024 * 1024;
	const download = $derived(id == null ? undefined : downloads.download(id));
	let text = $state('');
	let invalid = $state(false);

	$effect(() => {
		if (!open) return;
		const limit = download?.speedLimit;
		text = limit ? String(Math.round((limit / MB) * 100) / 100) : '';
		invalid = false;
	});

	async function save(limit: number | null) {
		if (id == null) return;
		await downloads.setSpeedLimit(id, limit);
		open = false;
	}

	function submit() {
		const value = text.trim().replace(',', '.');
		if (value === '') return save(null);
		const n = Number(value);
		if (!Number.isFinite(n) || n <= 0) {
			invalid = true;
			return;
		}
		void save(Math.round(n * MB));
	}
</script>

<Dialog bind:open title="Speed limit" width={400} onsubmit={submit}>
	<p>{download ? displayName(download) : ''}</p>
	<label class="field">
		<input
			class="textbox"
			type="text"
			inputmode="decimal"
			placeholder="No limit"
			aria-invalid={invalid}
			bind:value={text}
		/>
		MB/s
	</label>
	<small>Empty means only the global limit applies. A running plain download slows at once; yt-dlp and gallery-dl take it when they next start.</small>
	{#if invalid}<p class="error-text">Enter a speed above 0, or leave it empty.</p>{/if}

	{#snippet footer()}
		{#if download?.speedLimit}
			<button type="button" class="btn" onclick={() => save(null)}>Remove limit</button>
		{/if}
		<button type="button" class="btn ghost" onclick={() => (open = false)}>Cancel</button>
		<button type="submit" class="btn accent">Save</button>
	{/snippet}
</Dialog>

<style>
	p {
		margin: 0 0 var(--space-4);
		font: var(--type-body);
		color: var(--text-secondary);
		overflow-wrap: anywhere;
	}
	.field {
		display: flex;
		flex-direction: row;
		align-items: center;
		gap: var(--space-3);
		margin-bottom: var(--space-3);
		font: var(--type-body);
		color: var(--text);
	}
	.field input {
		width: 120px;
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
