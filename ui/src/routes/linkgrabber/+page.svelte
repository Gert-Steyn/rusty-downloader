<script lang="ts">
	import { goto } from '$app/navigation';
	import { open as pickFolder } from '@tauri-apps/plugin-dialog';
	import AvailabilityBadge from '#lib/components/AvailabilityBadge.svelte';
	import Dialog from '#lib/components/Dialog.svelte';
	import EditableText from '#lib/components/EditableText.svelte';
	import EmptyState from '#lib/components/EmptyState.svelte';
	import FormatPicker from '#lib/components/FormatPicker.svelte';
	import GalleryOptionsDialog from '#lib/components/GalleryOptionsDialog.svelte';
	import Icon from '#lib/components/Icon.svelte';
	import { isTauri, type GalleryDlOptions, type GrabbedLink, type YtDlpOptions } from '#lib/api.js';
	import { formatAgo, formatBytes, formatDuration, iconFor } from '#lib/format.js';
	import { DEFAULT_GALLERY_OPTIONS, changedCount, galleryLine, sameOptions } from '#lib/gallerydl.js';
	import { grabber, type LinkGroup } from '#lib/linkgrabber.svelte.js';

	let adding = $state(false);
	let addText = $state('');

	const checked = $derived(grabber.checkedIds.length);

	function groupState(group: LinkGroup) {
		const on = group.links.filter((l) => grabber.isChecked(l.id)).length;
		return { all: on === group.links.length, some: on > 0 && on < group.links.length };
	}

	function groupSize(group: LinkGroup): number | null {
		let total = 0;
		for (const l of group.links) {
			if (l.size == null) return null;
			total += l.size;
		}
		return total;
	}

	const DEFAULT_OPTIONS: YtDlpOptions = { preset: 'best', customFormat: null };

	function mediaLinks(group: LinkGroup): GrabbedLink[] {
		return group.links.filter((l) => l.kind === 'ytdlp');
	}

	function galleryLinks(group: LinkGroup): GrabbedLink[] {
		return group.links.filter((l) => l.kind === 'gallerydl');
	}

	// The gallery-dl options dialog, for one gallery or all of a package's.
	let galleryEdit = $state<{ ids: number[]; title: string; options: GalleryDlOptions; mixed: boolean } | null>(null);
	let galleryEditOpen = $state(false);

	function editGalleries(links: GrabbedLink[], title: string) {
		const first = links[0]?.gallerydl ?? DEFAULT_GALLERY_OPTIONS;
		galleryEdit = {
			ids: links.map((l) => l.id),
			title,
			options: first,
			mixed: links.some((l) => !sameOptions(l.gallerydl ?? DEFAULT_GALLERY_OPTIONS, first))
		};
		galleryEditOpen = true;
	}

	function optionsLabel(links: GrabbedLink[]): string {
		const changed = Math.max(...links.map((l) => changedCount(l.gallerydl)));
		return changed ? `Options (${changed})…` : 'Options…';
	}

	/** The group's shared choice, or `mixed` when its videos differ. */
	function groupFormat(links: GrabbedLink[]) {
		const first = links[0]?.ytdlp ?? DEFAULT_OPTIONS;
		const mixed = links.some(
			(l) => (l.ytdlp?.preset ?? 'best') !== first.preset || (l.ytdlp?.customFormat ?? null) !== first.customFormat
		);
		const heights = [...new Set(links.flatMap((l) => l.media?.heights ?? []))];
		return { options: first, mixed, heights };
	}

	function mediaLine(link: GrabbedLink): string {
		if (link.gallery) return galleryLine(link.gallery);
		const m = link.media;
		if (!m) return '';
		return [formatDuration(m.duration), m.uploader, m.isLive ? 'Live' : null].filter(Boolean).join(' · ');
	}

	async function changeFolder(group: LinkGroup) {
		const picked = await pickFolder({
			directory: true,
			defaultPath: group.destDir ?? undefined,
			title: `Folder for "${group.name}"`
		});
		if (typeof picked === 'string') await grabber.setFolder(group.links.map((l) => l.id), picked);
	}

	async function start() {
		if (await grabber.confirm()) goto('/');
	}

	async function submitAdd() {
		if (await grabber.add(addText, 'manual')) {
			adding = false;
			addText = '';
		}
	}
</script>

<div class="page">
	<header class="toolbar">
		<h1>LinkGrabber</h1>
		<label class="switch" title="Add links you copy anywhere to the LinkGrabber">
			<input
				type="checkbox"
				role="switch"
				checked={grabber.clipboardWatch}
				onchange={(e) => grabber.setClipboardWatch(e.currentTarget.checked)}
			/>
			Watch clipboard
		</label>
		<span class="spacer"></span>
		<button class="btn" onclick={() => (adding = true)}><Icon name="plus" /> Add links…</button>
		<button class="btn" disabled={!checked} onclick={() => grabber.recheck(grabber.checkedIds)}>Recheck</button>
		<button class="btn" disabled={!checked} onclick={() => grabber.remove(grabber.checkedIds)}>
			<Icon name="trash" /> Remove
		</button>
		<button class="btn" disabled={!grabber.links.length} onclick={grabber.clear}>Clear all</button>
		<button class="btn accent" disabled={!checked} onclick={start}>
			<Icon name="downloads" /> Start {checked || ''} download{checked === 1 ? '' : 's'}
		</button>
	</header>

	{#if grabber.error}
		<p class="banner" role="alert">{grabber.error}</p>
	{/if}

	{#if !grabber.ready}
		<div class="fill"></div>
	{:else if grabber.links.length === 0}
		<div class="fill">
			<EmptyState
				icon="link"
				title="LinkGrabber is empty"
				body={grabber.clipboardWatch
					? 'Copy links anywhere, paste them here with Ctrl+V, or drop them onto this window. They are checked before anything downloads.'
					: 'Paste links with Ctrl+V or drop them onto this window. Turn on "Watch clipboard" to pick up links you copy.'}
			/>
		</div>
	{:else}
		<div class="list">
			<div class="row head">
				<span></span>
				<span>Name</span>
				<span>Host</span>
				<span>Format</span>
				<span class="num">Size</span>
				<span>Availability</span>
			</div>
			{#each grabber.groups as group (group.key)}
				{@const st = groupState(group)}
				<section class="group" aria-label="Package {group.name}">
					<div class="row package">
						<input
							type="checkbox"
							aria-label="Download all of {group.name}"
							checked={st.all}
							indeterminate={st.some}
							onchange={(e) => grabber.setChecked(group.links.map((l) => l.id), e.currentTarget.checked)}
						/>
						<span class="name">
							<Icon name="package" />
							<EditableText
								value={group.name}
								label="Package name"
								hint="Package name: groups downloads in the list; it isn't a folder."
								onsave={(name) => grabber.setPackage(group.links.map((l) => l.id), name)}
							/>
							<span class="count">{group.links.length}</span>
						</span>
						<span class="folder" title={group.destDir ?? 'The folder chosen in Settings (Downloads unless changed)'}>
							{group.destDir ?? 'Default folder'}
						</span>
						<span class="format">
							{#if mediaLinks(group).length > 1}
								{@const media = mediaLinks(group)}
								{@const f = groupFormat(media)}
								<FormatPicker
									label="Format for all videos in {group.name}"
									options={f.options}
									mixed={f.mixed}
									heights={f.heights}
									onchange={(o) => grabber.setFormat(media.map((l) => l.id), o)}
								/>
							{:else if galleryLinks(group).length > 1}
								{@const galleries = galleryLinks(group)}
								<button
									class="btn small"
									title="gallery-dl options for every gallery in {group.name}"
									onclick={() => editGalleries(galleries, `Options for ${galleries.length} galleries`)}
								>
									{optionsLabel(galleries)}
								</button>
							{/if}
						</span>
						<span class="num">{formatBytes(groupSize(group))}</span>
						<span>
							<button class="btn small" onclick={() => changeFolder(group)} disabled={!isTauri()}>
								Change folder…
							</button>
						</span>
					</div>
					{#each group.links as link (link.id)}
						<div class="row link" class:off={!grabber.isChecked(link.id)}>
							<input
								type="checkbox"
								aria-label="Download {link.filename ?? link.url}"
								checked={grabber.isChecked(link.id)}
								onchange={(e) => grabber.setChecked([link.id], e.currentTarget.checked)}
							/>
							<span class="name" title={link.url}>
								<Icon name={iconFor(link.kind)} />
								<span class="label">
									{#if link.filename}
										<EditableText
											value={link.filename}
											label={link.kind === 'gallerydl' ? 'Folder name' : 'File name'}
											hint={link.kind === 'gallerydl' ? "Folder name: the gallery's files go into this folder." : undefined}
											onsave={(n) => grabber.rename(link.id, n)}
										/>
									{:else}
										<span class="pending">{link.url}</span>
									{/if}
									{#if link.kind === 'gallerydl' && link.filename}
										{@const path = `${group.destDir ?? 'Default folder'}\\${link.filename}`}
										<span class="meta dest" title="Saves to {path}">
											<Icon name="folder" size={12} /> Saves into a folder with this name
										</span>
									{/if}
									{#if mediaLine(link)}<span class="meta">{mediaLine(link)}</span>{/if}
									{#if link.previous}
										<span
											class="meta before"
											title={link.previous.exists
												? `Saved to ${link.previous.path}`
												: `Was saved to ${link.previous.path}, which is gone now`}
										>
											Downloaded {formatAgo(link.previous.finishedAt)}{link.previous.exists ? '' : ' (file gone)'}
										</span>
									{/if}
								</span>
							</span>
							<span
								class="host"
								title={link.media?.extractor
									? `${link.host} · ${link.media.extractor} (yt-dlp)`
									: link.gallery?.category
										? `${link.host} · ${link.gallery.category} (gallery-dl)`
										: link.host}
							>
								{link.host}
							</span>
							<span class="format">
								{#if link.kind === 'ytdlp'}
									<FormatPicker
										label="Format for {link.filename ?? link.url}"
										options={link.ytdlp ?? DEFAULT_OPTIONS}
										heights={link.media?.heights ?? []}
										onchange={(o) => grabber.setFormat([link.id], o)}
									/>
								{:else if link.kind === 'gallerydl'}
									<button
										class="btn small"
										title="gallery-dl options: selection, sign-in, impersonation, post-processing"
										onclick={() => editGalleries([link], `Options for ${link.filename ?? 'this gallery'}`)}
									>
										{optionsLabel([link])}
									</button>
								{/if}
							</span>
							<span class="num" title={link.kind === 'ytdlp' && link.size != null ? 'Estimated from the formats on offer' : undefined}>
								{link.kind === 'ytdlp' && link.size != null ? '~' : ''}{formatBytes(link.size)}
							</span>
							<span><AvailabilityBadge value={link.availability} error={link.error} /></span>
						</div>
					{/each}
				</section>
			{/each}
		</div>
	{/if}
</div>

{#if galleryEdit}
	{@const edit = galleryEdit}
	<GalleryOptionsDialog
		bind:open={galleryEditOpen}
		title={edit.title}
		options={edit.options}
		mixed={edit.mixed}
		onsave={(o) => grabber.setGalleryOptions(edit.ids, o)}
	/>
{/if}

<Dialog bind:open={adding} title="Add links to the LinkGrabber" width={560} onsubmit={submitAdd}>
	<label class="field">
		<span>Links</span>
		<textarea
			bind:value={addText}
			class="textbox mono"
			rows="8"
			placeholder="Paste text containing links. Each link is checked before you start it."
			spellcheck="false"
		></textarea>
	</label>
	{#snippet footer()}
		<button type="submit" class="btn accent" disabled={!addText.trim()}>Add</button>
		<button type="button" class="btn" onclick={() => (adding = false)}>Cancel</button>
	{/snippet}
</Dialog>

<style>
	.page {
		display: flex;
		flex-direction: column;
		height: 100%;
	}
	.toolbar {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 16px 20px 12px;
	}
	h1 {
		margin: 0 12px 0 0;
		font: 600 20px/28px var(--font-display);
	}
	.spacer,
	.fill {
		flex: 1;
	}
	.switch {
		display: inline-flex;
		align-items: center;
		gap: 8px;
		font-size: 13px;
		color: var(--text-secondary);
	}
	.banner {
		margin: 0 20px 12px;
		padding: 8px 16px;
		border-radius: var(--radius-control);
		background: color-mix(in srgb, var(--critical) 12%, transparent);
	}
	.list {
		flex: 1;
		overflow-y: auto;
		padding: 0 4px 16px;
	}
	.row {
		display: grid;
		grid-template-columns: 28px minmax(220px, 1fr) minmax(100px, 200px) 170px 88px 150px;
		align-items: center;
		gap: 12px;
		min-height: 32px;
		padding: 0 12px;
		font-size: 13px;
	}
	.head {
		position: sticky;
		top: 0;
		z-index: 1;
		font-size: 12px;
		color: var(--text-secondary);
		background: var(--page-fallback);
		border-bottom: 1px solid var(--stroke-divider);
	}
	:global([data-shell='tauri']) .head {
		background: transparent;
		backdrop-filter: blur(20px);
	}
	.group {
		margin-top: 6px;
	}
	.package {
		font-weight: 600;
		border-radius: var(--radius-control);
		background: var(--fill-subtle-hover);
	}
	.link {
		border-radius: var(--radius-control);
	}
	.link:hover {
		background: var(--fill-subtle-hover);
	}
	.link.off {
		color: var(--text-secondary);
	}
	.link .name {
		padding-left: 22px;
	}
	.name {
		display: flex;
		align-items: center;
		gap: 8px;
		min-width: 0;
	}
	.label {
		display: flex;
		flex: 1;
		flex-direction: column;
		min-width: 0;
	}
	.meta {
		font-size: 12px;
		font-weight: 400;
		color: var(--text-secondary);
	}
	.meta.dest {
		display: flex;
		align-items: center;
		gap: 4px;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.meta.dest :global(svg) {
		flex-shrink: 0;
	}
	.meta.before {
		color: var(--caution);
	}
	.format {
		min-width: 0;
		font-weight: 400;
	}
	.count {
		flex-shrink: 0;
		padding: 0 6px;
		border-radius: 8px;
		font-size: 11px;
		font-weight: 400;
		line-height: 16px;
		color: var(--text-secondary);
		background: var(--fill-subtle-hover);
	}
	.folder,
	.host,
	.pending {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		font-weight: 400;
		color: var(--text-secondary);
	}
	.pending {
		font-family: var(--font-mono);
		font-size: 12px;
	}
	.num {
		text-align: right;
		font-variant-numeric: tabular-nums;
		font-weight: 400;
	}
	.btn.small {
		min-height: 24px;
		padding: 2px 8px;
		font-size: 12px;
		font-weight: 400;
	}
	.mono {
		font-family: var(--font-mono);
		font-size: 12px;
	}
</style>
