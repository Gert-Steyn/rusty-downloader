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
		<button type="submit" class="btn accent">Save</button>
		{#if download?.speedLimit}
			<button type="button" class="btn" onclick={() => save(null)}>Remove limit</button>
		{/if}
		<button type="button" class="btn" onclick={() => (open = false)}>Cancel</button>
	{/snippet}
</Dialog>

<style>
	p {
		margin: 0 0 12px;
		overflow-wrap: anywhere;
	}
	.field {
		display: flex;
		align-items: center;
		gap: 8px;
		margin-bottom: 8px;
	}
	.field input {
		width: 120px;
	}
	small {
		display: block;
		font-size: 12px;
		color: var(--text-secondary);
	}
	.error-text {
		margin: 8px 0 0;
	}
</style>
