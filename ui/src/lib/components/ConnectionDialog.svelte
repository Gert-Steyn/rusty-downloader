<script lang="ts">
	// Which connection one download uses: the rules, directly, or a given proxy.
	import type { ProxyChoice } from '#lib/api.js';
	import { downloads } from '#lib/downloads.svelte.js';
	import { displayName } from '#lib/format.js';
	import { network } from '#lib/network.svelte.js';
	import Dialog from './Dialog.svelte';

	let { open = $bindable(false), id }: { open?: boolean; id: number | null } = $props();

	const download = $derived(id == null ? undefined : downloads.download(id));
	/** "auto", "direct" or a profile id, as the radio values. */
	let picked = $state('auto');

	const toValue = (c: ProxyChoice | undefined) => (c == null || c.mode === 'auto' ? 'auto' : c.mode === 'direct' ? 'direct' : String(c.id));
	const toChoice = (v: string): ProxyChoice =>
		v === 'auto' ? { mode: 'auto' } : v === 'direct' ? { mode: 'direct' } : { mode: 'profile', id: Number(v) };

	$effect(() => {
		if (!open) return;
		void network.load();
		picked = toValue(download?.proxy);
	});

	async function save() {
		if (id == null) return;
		await downloads.setProxy(id, toChoice(picked));
		open = false;
	}
</script>

<Dialog bind:open title="Connection" width={420} onsubmit={save}>
	<p>{download ? displayName(download) : ''}</p>
	<div class="choices" role="radiogroup" aria-label="Connection">
		<label><input type="radio" name="connection" value="auto" bind:group={picked} /> Follow the rules in Settings → Network</label>
		<label><input type="radio" name="connection" value="direct" bind:group={picked} /> Directly, without a proxy</label>
		{#each network.profiles as p (p.id)}
			<label><input type="radio" name="connection" value={String(p.id)} bind:group={picked} /> Through {p.name}</label>
		{/each}
	</div>
	<small>Applies when the download next starts; one that's running now keeps its connection until paused.</small>

	{#snippet footer()}
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
	.choices {
		display: grid;
		gap: var(--space-3);
		margin-bottom: var(--space-4);
		font: var(--type-body);
		color: var(--text);
	}
	.choices label {
		display: flex;
		align-items: center;
		gap: var(--space-3);
	}
	small {
		display: block;
		font: var(--type-small);
		color: var(--text-muted);
	}
</style>
