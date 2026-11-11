<script lang="ts">
	// gallery-dl options drawer: controls generated from the flag schema, with
	// flags the installed gallery-dl lacks hidden, plus a raw arguments box.
	import { untrack } from 'svelte';
	import type { FlagDef, GalleryDlFlags, GalleryDlOptions } from '#lib/api.js';
	import { DEFAULT_GALLERY_OPTIONS, galleryFlags } from '#lib/gallerydl.js';
	import Dialog from './Dialog.svelte';

	let {
		open = $bindable(false),
		title,
		options,
		mixed = false,
		onsave
	}: {
		open?: boolean;
		title: string;
		options: GalleryDlOptions;
		/** Several galleries with different settings: saving applies these to all. */
		mixed?: boolean;
		onsave: (options: GalleryDlOptions) => void;
	} = $props();

	let schema = $state<GalleryDlFlags | null>(null);
	let loadError = $state<string | null>(null);
	let draft = $state<GalleryDlOptions>(structuredClone(DEFAULT_GALLERY_OPTIONS));

	$effect(() => {
		if (!open) return;
		untrack(() => {
			draft = structuredClone($state.snapshot(options));
			if (!schema) {
				galleryFlags().then(
					(f) => (schema = f),
					(e) => (loadError = e instanceof Error ? e.message : String(e))
				);
			}
		});
	});

	const available = $derived(schema?.available ? new Set(schema.available) : null);
	const shown = (def: FlagDef) => !available || available.has(def.flag);
	const groups = $derived(
		(schema?.schema.groups ?? [])
			.map((g) => ({ ...g, flags: g.flags.filter(shown) }))
			.filter((g) => g.flags.length)
	);
	const hidden = $derived(
		(schema?.schema.groups ?? []).flatMap((g) => g.flags).filter((f) => !shown(f)).length
	);

	function value(id: string) {
		return draft.flags[id] ?? '';
	}

	function set(id: string, v: string) {
		if (v.trim() === '' || v === 'false') delete draft.flags[id];
		else draft.flags[id] = v;
	}

	/** Groups with something set start open, as do the commonly used ones. */
	function startsOpen(id: string, flags: FlagDef[]) {
		return id === 'selection' || id === 'impersonation' || flags.some((f) => value(f.id));
	}

	function save() {
		open = false;
		onsave({
			...$state.snapshot(draft),
			rawArgs: draft.rawArgs?.trim() || null,
			folderPattern: draft.folderPattern?.trim() || null
		});
	}
</script>

<Dialog bind:open {title} width={620} onsubmit={save}>
	<div class="scroll">
		{#if mixed}
			<p class="note">These galleries have different settings. Saving gives them all the settings below.</p>
		{/if}
		<label class="check">
			<input type="checkbox" bind:checked={draft.archive} />
			<span>
				Skip files downloaded from this site before
				<small>Uses a download archive per site, so getting a gallery again only fetches what's new.</small>
			</span>
		</label>
		<label class="check">
			<input type="checkbox" bind:checked={draft.userConfig} />
			<span>
				Use my gallery-dl config file
				<small>Off by default, so downloads behave the same on every machine.</small>
			</span>
		</label>
		<label class="check">
			<input type="checkbox" bind:checked={draft.keepFolders} />
			<span>
				Keep the site's folders
				<small>gallery-dl makes a folder per sub-gallery inside this gallery's folder.</small>
			</span>
		</label>
		{#if draft.keepFolders}
			<label class="field keep">
				<span>Folder pattern</span>
				<input
					class="textbox mono"
					spellcheck="false"
					placeholder={'{category}/{id}'}
					value={draft.folderPattern ?? ''}
					oninput={(e) => (draft.folderPattern = e.currentTarget.value)}
				/>
				<small class="hint">Optional. Folder levels separated by /, in gallery-dl's format. Empty uses gallery-dl's own layout.</small>
			</label>
		{:else}
			<p class="hint keep">
				All files go in one folder. Two sub-galleries with the same file names (001.jpg) keep only the first unless a filename pattern makes them unique.
			</p>
		{/if}

		{#if loadError}
			<p class="note error" role="alert">Could not load gallery-dl's options: {loadError}</p>
		{:else if !schema}
			<p class="note">Loading options…</p>
		{/if}

		{#each groups as group (group.id)}
			<details open={untrack(() => startsOpen(group.id, group.flags))}>
				<summary>{group.label}</summary>
				<div class="grid">
					{#each group.flags as def (def.id)}
						{@const id = `gdl-${def.id}`}
						{#if def.kind === 'switch'}
							<label class="check wide" title={def.help ?? undefined}>
								<input
									type="checkbox"
									checked={value(def.id) === 'true'}
									onchange={(e) => set(def.id, String(e.currentTarget.checked))}
								/>
								<span>{def.label} <code>{def.flag}{def.template ? ` ${def.template}` : ''}</code></span>
							</label>
						{:else}
							<label for={id} title={def.flag}>{def.label}</label>
							<div class="control">
								{#if def.kind === 'choice'}
									<select {id} class="select" value={value(def.id)} onchange={(e) => set(def.id, e.currentTarget.value)}>
										<option value="">Default</option>
										{#each def.choices as c (c.value)}
											<option value={c.value}>{c.label}</option>
										{/each}
									</select>
								{:else if def.kind === 'lines'}
									<textarea
										{id}
										class="textbox mono"
										rows="2"
										spellcheck="false"
										placeholder={def.placeholder ?? ''}
										value={value(def.id)}
										oninput={(e) => set(def.id, e.currentTarget.value)}
									></textarea>
								{:else}
									<input
										{id}
										class="textbox"
										class:mono={def.kind === 'text'}
										type={def.kind === 'number' ? 'number' : 'text'}
										min={def.kind === 'number' ? 0 : undefined}
										spellcheck="false"
										placeholder={def.placeholder ?? ''}
										value={value(def.id)}
										oninput={(e) => set(def.id, e.currentTarget.value)}
									/>
								{/if}
								{#if def.help}<small>{def.help}</small>{/if}
							</div>
						{/if}
					{/each}
				</div>
			</details>
		{/each}

		<label class="field raw">
			<span>Extra arguments</span>
			<input
				class="textbox mono"
				spellcheck="false"
				placeholder="--sleep-429 60 -o &quot;extractor.twitter.videos=false&quot;"
				value={draft.rawArgs ?? ''}
				oninput={(e) => (draft.rawArgs = e.currentTarget.value)}
			/>
			<small>Passed to gallery-dl as typed; quotes group words.</small>
		</label>
		{#if hidden}
			<p class="note">
				{hidden} option{hidden === 1 ? " isn't" : "s aren't"} supported by the installed gallery-dl and
				{hidden === 1 ? 'is' : 'are'} hidden.
			</p>
		{/if}
	</div>

	{#snippet footer()}
		<button
			type="button"
			class="btn ghost reset"
			onclick={() => (draft = structuredClone(DEFAULT_GALLERY_OPTIONS))}
		>
			Reset to defaults
		</button>
		<button type="button" class="btn ghost" onclick={() => (open = false)}>Cancel</button>
		<button type="submit" class="btn accent">Save</button>
	{/snippet}
</Dialog>

<style>
	.scroll {
		max-height: min(60vh, 560px);
		overflow-y: auto;
	}
	.note {
		margin: var(--space-3) 0;
		font: var(--type-body);
		color: var(--text-secondary);
	}
	.note.error {
		color: var(--danger);
	}
	.check {
		display: flex;
		align-items: flex-start;
		gap: var(--space-3);
		margin: 0 0 var(--space-4);
		font: var(--type-body);
	}
	.check input {
		margin-top: var(--space-1);
	}
	.check span {
		display: flex;
		flex-direction: column;
		gap: var(--space-1);
	}
	.check.wide {
		grid-column: 1 / -1;
		margin: 0;
	}
	.check.wide span {
		display: inline;
	}
	small {
		font: var(--type-small);
		color: var(--text-muted);
	}
	code {
		font: var(--type-mono);
		color: var(--text-muted);
	}
	details {
		margin: var(--space-3) 0;
		border-top: 1px solid var(--border-subtle);
		padding-top: var(--space-3);
	}
	summary {
		cursor: default;
		font: var(--type-heading);
		padding: var(--space-2) 0;
	}
	.grid {
		display: grid;
		grid-template-columns: 170px 1fr;
		align-items: start;
		gap: var(--space-3) var(--space-4);
		margin: var(--space-3) 0 var(--space-2);
		font: var(--type-body);
	}
	.grid > label:not(.check) {
		padding-top: var(--space-2);
		font: var(--type-label);
		color: var(--text);
	}
	.control {
		display: flex;
		flex-direction: column;
		gap: var(--space-1);
		min-width: 0;
	}
	.raw {
		margin-top: var(--space-4);
		border-top: 1px solid var(--border-subtle);
		padding-top: var(--space-4);
	}
	.keep {
		margin: var(--space-3) 0 var(--space-4) var(--space-6);
	}
	.reset {
		margin-right: auto;
	}
</style>
