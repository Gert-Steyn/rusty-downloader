<script lang="ts" module>
	/** Pull distinct http(s) and ftp(s) URLs out of pasted text, dropping trailing punctuation. */
	export function extractLinks(text: string): string[] {
		const found = text.match(/(?:https?|ftps?):\/\/[^\s"'<>]+/gi) ?? [];
		return [...new Set(found.map((u) => u.replace(/[),.;:!?\]]+$/, '')))];
	}
</script>

<script lang="ts">
	import { untrack } from 'svelte';
	import { open as pickFolder } from '@tauri-apps/plugin-dialog';
	import { commands, isTauri, unwrap } from '#lib/api.js';
	import { downloads } from '#lib/downloads.svelte.js';
	import Dialog from './Dialog.svelte';

	let {
		open = $bindable(false),
		initialText = '',
		onadded
	}: { open?: boolean; initialText?: string; onadded?: (packageId: number) => void } = $props();

	let text = $state('');
	let packageName = $state('');
	let destDir = $state('');
	let busy = $state(false);
	let error = $state<string | null>(null);
	let textarea: HTMLTextAreaElement | undefined = $state();

	const links = $derived(extractLinks(text));

	// Reset when the dialog opens. Only `open` may be a dependency: anything else
	// read here (like `destDir`) would re-run this and wipe what was typed.
	$effect(() => {
		if (!open) return;
		untrack(() => {
			text = initialText;
			error = null;
			if (!destDir && isTauri()) {
				unwrap(commands.defaultDownloadDir()).then((dir) => (destDir ||= dir));
			}
		});
		queueMicrotask(() => textarea?.focus());
	});

	async function browse() {
		const picked = await pickFolder({
			directory: true,
			defaultPath: destDir || undefined,
			title: 'Choose a download folder'
		});
		if (typeof picked === 'string') destDir = picked;
	}

	async function submit() {
		if (!links.length || busy) return;
		busy = true;
		error = null;
		try {
			const result = await unwrap(
				commands.addLinks({ urls: links, packageName: packageName || null, destDir: destDir || null })
			);
			open = false;
			text = '';
			packageName = '';
			onadded?.(result.packageId);
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		} finally {
			busy = false;
		}
	}
</script>

<Dialog bind:open title="Add links" width={600} onsubmit={submit}>
	<label class="field">
		<span>Links</span>
		<textarea
			bind:this={textarea}
			bind:value={text}
			class="textbox mono"
			rows="7"
			placeholder="Paste one or more links. Anything else in the text is ignored."
			spellcheck="false"
		></textarea>
		<span class="hint">
			{links.length === 0 ? 'No links found yet' : `${links.length} link${links.length === 1 ? '' : 's'} found`}
		</span>
	</label>

	<label class="field">
		<span>Package name</span>
		<input class="textbox" bind:value={packageName} placeholder="Named after the links if left empty" />
	</label>

	<div class="field">
		<label for="dest-dir">Save to</label>
		<div class="row">
			<input id="dest-dir" class="textbox" bind:value={destDir} placeholder="Downloads folder" />
			<button type="button" class="btn" onclick={browse} disabled={!isTauri()}>Browse…</button>
		</div>
	</div>

	{#if error}<p class="error-text" role="alert">{error}</p>{/if}

	{#snippet footer()}
		<button type="submit" class="btn accent" disabled={!links.length || busy}>
			{busy ? 'Adding…' : 'Start downloads'}
		</button>
		<button type="button" class="btn" onclick={() => (open = false)}>Cancel</button>
	{/snippet}
</Dialog>

<style>
	.row {
		display: flex;
		gap: 8px;
	}
	.mono {
		font-family: var(--font-mono);
		font-size: 12px;
		line-height: 18px;
	}
	.error-text {
		margin: 0;
	}
</style>
