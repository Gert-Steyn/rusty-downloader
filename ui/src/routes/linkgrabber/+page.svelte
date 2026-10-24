<script lang="ts">
	import { goto } from '$app/navigation';
	import { open as pickFolder } from '@tauri-apps/plugin-dialog';
	import AvailabilityBadge from '#lib/components/AvailabilityBadge.svelte';
	import Dialog from '#lib/components/Dialog.svelte';
	import EditableText from '#lib/components/EditableText.svelte';
	import EmptyState from '#lib/components/EmptyState.svelte';
	import Icon from '#lib/components/Icon.svelte';
	import { isTauri } from '#lib/api.js';
	import { formatBytes } from '#lib/format.js';
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
							<Icon name="folder" />
							<EditableText
								value={group.name}
								label="Package name"
								onsave={(name) => grabber.setPackage(group.links.map((l) => l.id), name)}
							/>
							<span class="count">{group.links.length}</span>
						</span>
						<span class="folder" title={group.destDir ?? 'Downloads folder'}>
							{group.destDir ?? 'Downloads folder'}
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
								<Icon name="file" />
								{#if link.filename}
									<EditableText value={link.filename} label="File name" onsave={(n) => grabber.rename(link.id, n)} />
								{:else}
									<span class="pending">{link.url}</span>
								{/if}
							</span>
							<span class="host">{link.host}</span>
							<span class="num">{formatBytes(link.size)}</span>
							<span><AvailabilityBadge value={link.availability} error={link.error} /></span>
						</div>
					{/each}
				</section>
			{/each}
		</div>
	{/if}
</div>

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
		grid-template-columns: 28px minmax(220px, 1fr) minmax(120px, 220px) 88px 150px;
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
