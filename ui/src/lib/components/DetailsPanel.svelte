<script lang="ts">
	import { downloads } from '#lib/downloads.svelte.js';
	import {
		STATUS_LABELS,
		displayName,
		formatBytes,
		formatEta,
		formatSpeed,
		percent,
		summarize
	} from '#lib/format.js';
	import { changedCount } from '#lib/gallerydl.js';
	import { presetLabel } from '#lib/media.js';
	import Icon from './Icon.svelte';
	import type { Selection } from './DownloadsTable.svelte';

	let { selection, onremove }: { selection: Selection; onremove: () => void } = $props();

	const download = $derived(selection?.kind === 'download' ? downloads.download(selection.id) : undefined);
	const pkg = $derived(
		selection?.kind === 'package'
			? downloads.package(selection.id)
			: download
				? downloads.package(download.packageId)
				: undefined
	);
	const live = $derived(download ? downloads.live[download.id] : undefined);
	const canPause = $derived(download?.status === 'active' || download?.status === 'queued');
	const canResume = $derived(
		download?.status === 'paused' || download?.status === 'failed' || download?.status === 'canceled'
	);
</script>

<section class="details" aria-label="Details">
	{#if download && pkg}
		{@const pct = percent(download.downloaded, download.size)}
		<header>
			<div class="title">
				<h2 class="selectable">{displayName(download)}</h2>
				<span class="status {download.status}">{STATUS_LABELS[download.status]}</span>
			</div>
			<div class="actions">
				{#if canPause}
					<button class="btn" onclick={() => downloads.pause(download.id)}><Icon name="pause" /> Pause</button>
				{:else if canResume}
					<button class="btn" onclick={() => downloads.resume(download.id)}>
						<Icon name="play" /> {download.status === 'paused' ? 'Resume' : 'Retry'}
					</button>
				{/if}
				{#if download.status === 'completed'}
					<button class="btn" onclick={() => downloads.open(download.id)}><Icon name="open" /> Open</button>
				{/if}
				<button class="btn" onclick={() => downloads.reveal(download.id)}><Icon name="folder" /> Show in folder</button>
				<button class="btn" onclick={onremove}><Icon name="trash" /> Remove</button>
			</div>
		</header>
		{#if download.error}
			<p class="error selectable">{download.error}</p>
		{/if}
		<dl>
			<dt>Progress</dt>
			<dd>
				{#if download.files}
					{download.files.done} of {download.files.total ?? 'an unknown number of'} files ·
					{formatBytes(download.downloaded)}{#if download.size != null && download.status !== 'completed'}
						of about {formatBytes(download.size)}{/if}
				{:else}
					{formatBytes(download.downloaded)} of {formatBytes(download.size)}
					{#if pct != null}({pct.toFixed(1)}%){/if}
				{/if}
			</dd>
			<dt>Speed</dt>
			<dd>
				{download.status === 'active' ? formatSpeed(live?.speedBps) : '—'}
				{#if live?.connections}· {live.connections} connection{live.connections === 1 ? '' : 's'}{/if}
			</dd>
			<dt>Time left</dt>
			<dd>
				{download.status === 'active'
					? formatEta(download.size == null ? null : download.size - download.downloaded, live?.speedBps ?? 0)
					: '—'}
			</dd>
			<dt>Package</dt>
			<dd>{pkg.name}</dd>
			{#if download.ytdlp}
				<dt>Format</dt>
				<dd>{presetLabel(download.ytdlp)} · yt-dlp</dd>
			{/if}
			{#if download.gallerydl}
				{@const changed = changedCount(download.gallerydl)}
				<dt>Options</dt>
				<dd>
					gallery-dl · {changed ? `${changed} setting${changed === 1 ? '' : 's'} changed` : 'defaults'}
					{#if download.gallerydl.archive}· skips files fetched before{/if}
				</dd>
			{/if}
			<dt>Source</dt>
			<dd class="wide mono selectable">{download.sourceUrl}</dd>
			<dt>{download.files ? 'Folder' : 'Saved to'}</dt>
			<dd class="wide mono selectable">{download.destPath ?? `${pkg.destDir} (name not known yet)`}</dd>
		</dl>
	{:else if pkg}
		{@const s = summarize(pkg)}
		{@const active = pkg.downloads.some((d) => d.status === 'active' || d.status === 'queued')}
		<header>
			<div class="title">
				<h2 class="selectable">{pkg.name}</h2>
				<span class="status {s.status}">{s.completed}/{pkg.downloads.length} done</span>
			</div>
			<div class="actions">
				{#if active}
					<button class="btn" onclick={() => pkg.downloads.forEach((d) => downloads.pause(d.id))}>
						<Icon name="pause" /> Pause all
					</button>
				{:else if s.status !== 'completed'}
					<button class="btn" onclick={() => pkg.downloads.forEach((d) => downloads.resume(d.id))}>
						<Icon name="play" /> Resume all
					</button>
				{/if}
				<button class="btn" onclick={() => downloads.openFolder(pkg.id)}><Icon name="folder" /> Open folder</button>
				<button class="btn" onclick={onremove}><Icon name="trash" /> Remove</button>
			</div>
		</header>
		<dl>
			<dt>Progress</dt>
			<dd>{formatBytes(s.downloaded)} of {formatBytes(s.size)}</dd>
			<dt>Files</dt>
			<dd>{pkg.downloads.length}</dd>
			<dt>Folder</dt>
			<dd class="wide mono selectable">{pkg.destDir}</dd>
		</dl>
	{:else}
		<p class="empty">Select a download to see its details.</p>
	{/if}
</section>

<style>
	.details {
		height: 180px;
		flex-shrink: 0;
		overflow: auto;
		padding: 12px 20px 16px;
		border-top: 1px solid var(--stroke-divider);
	}
	header {
		display: flex;
		align-items: center;
		gap: 16px;
		margin-bottom: 8px;
	}
	.title {
		display: flex;
		align-items: baseline;
		gap: 12px;
		min-width: 0;
		flex: 1;
	}
	h2 {
		margin: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		font: 600 16px/22px var(--font-display);
	}
	.actions {
		display: flex;
		gap: 8px;
		flex-shrink: 0;
	}
	.status {
		flex-shrink: 0;
		font-size: 12px;
		color: var(--text-secondary);
	}
	.status.completed {
		color: var(--success);
	}
	.status.failed {
		color: var(--critical);
	}
	.status.active {
		color: var(--accent-text-on-layer);
	}
	.error {
		margin: 0 0 8px;
		color: var(--critical);
	}
	/* Two columns of short facts; long values (URLs, paths) span the row. */
	dl {
		display: grid;
		grid-template-columns: 88px minmax(0, 1fr) 88px minmax(0, 1fr);
		gap: 4px 16px;
		margin: 0;
		font-size: 13px;
	}
	dt {
		color: var(--text-secondary);
	}
	dd {
		margin: 0;
		min-width: 0;
		overflow-wrap: anywhere;
	}
	.wide {
		grid-column: 2 / -1;
	}
	.mono {
		font-family: var(--font-mono);
		font-size: 12px;
	}
	.empty {
		margin: 0;
		color: var(--text-secondary);
	}
</style>
