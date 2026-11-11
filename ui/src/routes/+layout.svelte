<script lang="ts">
	import '@fontsource/inter/400.css';
	import '@fontsource/inter/500.css';
	import '@fontsource/inter/600.css';
	import '@fontsource/jetbrains-mono/400.css';
	import '#lib/styles/graphite.css';
	import '#lib/styles/theme.css';
	import { applyTheme, loadTheme } from '#lib/theme.js';
	import favicon from '#lib/assets/favicon.png';
	import Icon from '#lib/components/Icon.svelte';
	import Toast from '#lib/components/Toast.svelte';
	import { mainNav, footerNav, type NavItem } from '#lib/nav.js';
	import { commands, isTauri, unwrap, type LinkSource } from '#lib/api.js';
	import { downloads } from '#lib/downloads.svelte.js';
	import { isLinkDrag, isTorrentFile, textFromDrop } from '#lib/drop.js';
	import { formatSpeed } from '#lib/format.js';
	import { grabber } from '#lib/linkgrabber.svelte.js';
	import { network } from '#lib/network.svelte.js';
	import { speed } from '#lib/speed.svelte.js';
	import { countdown, countdownText } from '#lib/countdown.svelte.js';
	import { queues } from '#lib/queues.svelte.js';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();

	applyTheme(loadTheme());

	let version = $state<string | null>(null);

	if (isTauri()) {
		document.documentElement.dataset.shell = 'tauri';
		unwrap(commands.appInfo())
			.then((info) => (version = info.version))
			.catch((e) => console.error('app_info failed', e));
	}
	// Owned by the layout so the status bar and badges are live on every page.
	downloads.start();
	grabber.start();
	speed.start();
	void network.start();
	queues.start();
	countdown.start();

	const limitText = $derived.by(() => {
		const { limit, source, until } = speed.active;
		if (source === 'none' || limit == null) return 'No speed limit';
		const text = `Limited to ${formatSpeed(limit)}`;
		return source === 'schedule' && until ? `${text} until ${until}` : text;
	});
	const limitHint = $derived(
		speed.active.source === 'manual'
			? 'Click to lift the limit (scheduled limits still apply)'
			: speed.active.source === 'schedule'
				? 'A scheduled limit. Click to use your own limit instead'
				: 'Click to limit the speed (the limit is set in Settings)'
	);

	const isActive = (item: NavItem) =>
		item.href === '/' ? page.url.pathname === '/' : page.url.pathname.startsWith(item.href);

	/** Text that may hold links: web links, or a magnet link. */
	const LINK_HINT = /https?:\/\/|magnet:\?/i;

	/** Links go to the LinkGrabber first, wherever they come from. */
	async function grab(text: string, source: LinkSource) {
		if ((await grabber.add(text, source)) && page.url.pathname !== '/linkgrabber') goto('/linkgrabber');
	}

	function busyElsewhere(target: EventTarget | null): boolean {
		const inField = target instanceof HTMLElement && target.closest('input, textarea, [contenteditable]');
		return Boolean(inField) || document.querySelector('dialog[open]') !== null;
	}

	// Ctrl+V anywhere outside a text field stages the pasted links.
	function onpaste(e: ClipboardEvent) {
		if (busyElsewhere(e.target)) return;
		const text = e.clipboardData?.getData('text') ?? '';
		if (!LINK_HINT.test(text)) return;
		e.preventDefault();
		grab(text, 'manual');
	}

	// Count enter/leave pairs: dragleave also fires when crossing child elements.
	let dragDepth = $state(0);
	const dropping = $derived(dragDepth > 0);

	function ondragenter(e: DragEvent) {
		if (!isLinkDrag(e.dataTransfer) || document.querySelector('dialog[open]')) return;
		e.preventDefault();
		dragDepth++;
	}
	function ondragover(e: DragEvent) {
		if (dragDepth > 0) {
			e.preventDefault();
			if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
		}
	}
	function ondragleave() {
		if (dragDepth > 0) dragDepth--;
	}
	async function ondrop(e: DragEvent) {
		if (dragDepth === 0 || !e.dataTransfer) return;
		e.preventDefault();
		dragDepth = 0;
		// Read the files before the first await: the drop's file list goes away with the event.
		const torrents = Array.from(e.dataTransfer.files).filter(isTorrentFile);
		const text = await textFromDrop(e.dataTransfer);
		for (const file of torrents) await grabber.addTorrentFile(file);
		if (torrents.length && page.url.pathname !== '/linkgrabber') goto('/linkgrabber');
		if (LINK_HINT.test(text)) grab(text, 'drop');
		else if (!torrents.length) grabber.say('No links found in what was dropped');
	}
</script>

<svelte:window {onpaste} {ondragenter} {ondragover} {ondragleave} {ondrop} />

<svelte:head>
	<link rel="icon" href={favicon} />
</svelte:head>

{#snippet navLink(item: NavItem)}
	<a href={item.href} class="nav-item" class:active={isActive(item)} aria-current={isActive(item) ? 'page' : undefined}>
		<Icon name={item.icon} />
		<span>{item.label}</span>
		{#if item.href === '/linkgrabber' && grabber.links.length}
			<span class="tag accent badge" aria-label="{grabber.links.length} links waiting">{grabber.links.length}</span>
		{/if}
	</a>
{/snippet}

<div class="shell">
	<nav class="sidebar" aria-label="Main">
		<div class="brand">
			Rusty Downloader
			{#if version}<span class="version">v{version}</span>{/if}
		</div>
		<div class="nav-list">
			{#each mainNav as item (item.href)}{@render navLink(item)}{/each}
		</div>
		<div class="nav-list footer">
			{#each footerNav as item (item.href)}{@render navLink(item)}{/each}
		</div>
	</nav>

	<main class="content">
		{#if countdown.pending}
			<div class="banner info countdown" role="alert">
				<Icon name="queues" />
				<span>
					<strong>{countdown.pending.queueName} has finished.</strong>
					{countdownText(countdown.pending.action, countdown.secondsLeft)}.
				</span>
				<button type="button" class="btn ghost small" onclick={() => countdown.cancel()}>Cancel</button>
			</div>
		{/if}
		{@render children()}
	</main>

	<footer class="statusbar">
		<span><Icon name="downloads" /> {downloads.stats.speed ? formatSpeed(downloads.stats.speed) : '0 B/s'}</span>
		<span>{downloads.stats.active} active</span>
		<span>{downloads.stats.queued} queued</span>
		{#if downloads.stats.seeding}
			<span><Icon name="upload" /> {downloads.stats.seeding} seeding · {downloads.stats.upload ? formatSpeed(downloads.stats.upload) : '0 B/s'}</span>
		{/if}
		<span class="spacer"></span>
		{#if network.status.state === 'down'}
			<a class="net down" href="/settings" title="Settings → Network">
				<Icon name="network" />
				{network.status.adapter} is down{network.status.held ? ' · downloads paused' : ''}
			</a>
		{:else if network.status.state === 'up'}
			<a class="net" href="/settings" title="All connections go through {network.status.adapter} ({network.status.address})">
				<Icon name="network" />
				Via {network.status.adapter}
			</a>
		{/if}
		<button
			type="button"
			class="limit"
			class:on={speed.active.source !== 'none'}
			aria-pressed={speed.active.source === 'manual'}
			title={speed.error ?? limitHint}
			onclick={() => speed.toggle()}
		>
			<Icon name="speed" />
			{limitText}
		</button>
	</footer>
</div>

{#if dropping}
	<div class="drop-overlay" aria-hidden="true">
		<div class="drop-card">
			<Icon name="link" size={24} />
			<strong>Drop to add links</strong>
			<span>Links, selected text, link-list files and .torrent files go to the LinkGrabber</span>
		</div>
	</div>
{/if}

<Toast
	message={grabber.notice}
	action={grabber.notice?.addText
		? 'Add'
		: grabber.notice?.review && page.url.pathname !== '/linkgrabber'
			? 'Review'
			: undefined}
	duration={grabber.notice?.addText ? 10000 : 4000}
	onaction={() => {
		const text = grabber.notice?.addText;
		if (text) grab(text, 'clipboard');
		else goto('/linkgrabber');
	}}
/>

	<style>
		.shell {
			display: grid;
			grid-template-columns: var(--sidebar-w) 1fr;
			grid-template-rows: 1fr auto;
			height: 100vh;
		}

		.sidebar {
			grid-row: 1 / 3;
			display: flex;
			flex-direction: column;
			padding: var(--space-3);
			background: var(--surface);
			border-right: 1px solid var(--border-subtle);
		}
		.brand {
			padding: var(--space-2) var(--space-3) var(--space-5);
			font: var(--type-heading);
			color: var(--text);
		}
		.version {
			margin-left: var(--space-2);
			font: var(--type-small);
			color: var(--text-muted);
		}
		.nav-list {
			display: flex;
			flex-direction: column;
			gap: var(--space-1);
		}
		.nav-list.footer {
			margin-top: auto;
		}
		.nav-item {
			display: flex;
			align-items: center;
			gap: var(--space-3);
			height: var(--row-h);
			padding: 0 var(--space-3);
			border-radius: var(--radius-md);
			font: var(--type-label);
			color: var(--text-secondary);
			text-decoration: none;
			cursor: default;
			transition:
				background var(--motion-fast) var(--ease),
				color var(--motion-fast) var(--ease);
		}
		.nav-item > :global(svg) {
			flex: none;
			color: var(--text-muted);
			transition: color var(--motion-fast) var(--ease);
		}
		.nav-item:hover {
			color: var(--text);
			background: var(--hover);
		}
		.nav-item:hover > :global(svg) {
			color: var(--text);
		}
		.nav-item:active {
			background: var(--pressed);
		}
		.nav-item.active {
			color: var(--text);
			background: var(--hover);
		}
		.nav-item.active > :global(svg) {
			color: var(--accent);
		}

		.content {
			overflow: auto;
			background: var(--canvas);
		}

		.statusbar {
			display: flex;
			align-items: center;
			gap: var(--space-5);
			height: 28px;
			padding: 0 var(--space-4);
			font: var(--type-small);
			color: var(--text-muted);
			background: var(--canvas);
			border-top: 1px solid var(--border-subtle);
		}
		.statusbar span {
			display: inline-flex;
			align-items: center;
			gap: var(--space-2);
		}
		.statusbar .spacer {
			flex: 1;
		}
		.countdown {
			margin: var(--space-5) var(--space-5) 0;
		}
		.countdown span {
			flex: 1;
		}
		.statusbar .limit {
			display: inline-flex;
			align-items: center;
			gap: var(--space-2);
			height: var(--control-h-sm);
			padding: 0 var(--space-3);
			border: 0;
			border-radius: var(--radius-md);
			font: var(--type-small);
			color: inherit;
			background: transparent;
			cursor: default;
			transition:
				background var(--motion-fast) var(--ease),
				color var(--motion-fast) var(--ease);
		}
		.statusbar .limit:hover:not(:disabled) {
			color: var(--text);
			background: var(--hover);
		}
		.statusbar .limit:active:not(:disabled) {
			background: var(--pressed);
		}
		.statusbar .limit.on {
			color: var(--accent);
		}
		.statusbar .net {
			display: inline-flex;
			align-items: center;
			gap: var(--space-2);
			color: inherit;
			text-decoration: none;
		}
		.statusbar .net.down {
			color: var(--danger);
		}

		.badge {
			margin-left: auto;
		}

		.drop-overlay {
			position: fixed;
			inset: 0;
			z-index: 20;
			display: grid;
			place-items: center;
			background: var(--scrim);
			border: 1px dashed var(--accent);
			pointer-events: none;
		}
		.drop-card {
			display: flex;
			flex-direction: column;
			align-items: center;
			gap: var(--space-3);
			padding: var(--space-6) var(--space-7);
			border: 1px solid var(--border);
			border-radius: var(--radius-xl);
			background: var(--raised);
			color: var(--text-secondary);
			box-shadow: var(--shadow-popover);
		}
		.drop-card > :global(svg) {
			color: var(--text-muted);
		}
		.drop-card strong {
			font: var(--type-heading);
			color: var(--text);
		}
	</style>
