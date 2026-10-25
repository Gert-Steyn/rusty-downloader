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
		onsave({ ...$state.snapshot(draft), rawArgs: draft.rawArgs?.trim() || null });
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
			class="btn reset"
			onclick={() => (draft = structuredClone(DEFAULT_GALLERY_OPTIONS))}
		>
			Reset to defaults
		</button>
		<button type="submit" class="btn accent">Save</button>
		<button type="button" class="btn" onclick={() => (open = false)}>Cancel</button>
	{/snippet}
</Dialog>

<style>
	.scroll {
		max-height: min(60vh, 560px);
		overflow-y: auto;
		margin: 0 -24px;
		padding: 0 24px;
	}
	.note {
		margin: 8px 0;
		font-size: 13px;
		color: var(--text-secondary);
	}
	.note.error {
		color: var(--critical);
	}
	.check {
		display: flex;
		align-items: flex-start;
		gap: 8px;
		margin: 0 0 10px;
		font-size: 13px;
	}
	.check input {
		margin-top: 2px;
	}
	.check span {
		display: flex;
		flex-direction: column;
		gap: 2px;
	}
	.check.wide {
		grid-column: 1 / -1;
		margin: 0;
	}
	.check.wide span {
		display: inline;
	}
	small {
		font-size: 12px;
		color: var(--text-secondary);
	}
	code {
		font-family: var(--font-mono);
		font-size: 11px;
		color: var(--text-secondary);
	}
	details {
		margin: 8px 0;
		border-top: 1px solid var(--stroke-divider);
		padding-top: 8px;
	}
	summary {
		cursor: default;
		font-weight: 600;
		font-size: 14px;
		padding: 4px 0;
	}
	.grid {
		display: grid;
		grid-template-columns: 170px 1fr;
		align-items: start;
		gap: 8px 12px;
		margin: 8px 0 4px;
		font-size: 13px;
	}
	.grid > label:not(.check) {
		padding-top: 6px;
		color: var(--text-secondary);
	}
	.control {
		display: flex;
		flex-direction: column;
		gap: 2px;
		min-width: 0;
	}
	.raw {
		margin-top: 12px;
		border-top: 1px solid var(--stroke-divider);
		padding-top: 12px;
	}
	.mono {
		font-family: var(--font-mono);
		font-size: 12px;
	}
	.reset {
		margin-right: auto;
	}
</style>
