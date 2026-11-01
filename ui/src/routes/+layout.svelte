<script lang="ts">
	import '#lib/styles/theme.css';
	import favicon from '#lib/assets/favicon.svg';
	import Icon from '#lib/components/Icon.svelte';
	import Toast from '#lib/components/Toast.svelte';
	import { mainNav, footerNav, type NavItem } from '#lib/nav.js';
	import { commands, isTauri, unwrap, type LinkSource } from '#lib/api.js';
	import { downloads } from '#lib/downloads.svelte.js';
	import { isLinkDrag, textFromDrop } from '#lib/drop.js';
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
		if (!/https?:\/\//i.test(text)) return;
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
		const text = await textFromDrop(e.dataTransfer);
		if (/https?:\/\//i.test(text)) grab(text, 'drop');
		else grabber.say('No links found in what was dropped');
	}
</script>

<svelte:window {onpaste} {ondragenter} {ondragover} {ondragleave} {ondrop} />

<svelte:head>
	<link rel="icon" href={favicon} />
</svelte:head>

{#snippet navLink(item: NavItem)}
	<a href={item.href} class="nav-item" class:active={isActive(item)} aria-current={isActive(item) ? 'page' : undefined}>
		<Icon name={item.icon} size={18} />
		<span>{item.label}</span>
		{#if item.href === '/linkgrabber' && grabber.links.length}
			<span class="badge" aria-label="{grabber.links.length} links waiting">{grabber.links.length}</span>
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
			<div class="countdown" role="alert">
				<Icon name="queues" size={18} />
				<span>
					<strong>{countdown.pending.queueName} has finished.</strong>
					{countdownText(countdown.pending.action, countdown.secondsLeft)}.
				</span>
				<button type="button" class="btn accent" onclick={() => countdown.cancel()}>Cancel</button>
			</div>
		{/if}
		{@render children()}
	</main>

	<footer class="statusbar">
		<span><Icon name="downloads" size={14} /> {downloads.stats.speed ? formatSpeed(downloads.stats.speed) : '0 B/s'}</span>
		<span>{downloads.stats.active} active</span>
		<span>{downloads.stats.queued} queued</span>
		<span class="spacer"></span>
		{#if network.status.state === 'down'}
			<a class="net down" href="/settings" title="Settings → Network">
				<Icon name="network" size={14} />
				{network.status.adapter} is down{network.status.held ? ' · downloads paused' : ''}
			</a>
		{:else if network.status.state === 'up'}
			<a class="net" href="/settings" title="All connections go through {network.status.adapter} ({network.status.address})">
				<Icon name="network" size={14} />
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
			<Icon name="speed" size={14} />
			{limitText}
		</button>
	</footer>
</div>

{#if dropping}
	<div class="drop-overlay" aria-hidden="true">
		<div class="drop-card">
			<Icon name="link" size={28} />
			<strong>Drop to add links</strong>
			<span>Links, selected text and link-list files go to the LinkGrabber</span>
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
		grid-template-columns: 220px 1fr;
		grid-template-rows: 1fr auto;
		height: 100vh;
	}

	.sidebar {
		grid-row: 1 / 3;
		display: flex;
		flex-direction: column;
		padding: 12px 4px 8px;
	}
	.brand {
		padding: 4px 12px 16px;
		font: 600 14px/20px var(--font-display);
	}
	.version {
		margin-left: 4px;
		font: 400 12px/16px var(--font-ui);
		color: var(--text-secondary);
	}
	.nav-list {
		display: flex;
		flex-direction: column;
		gap: 2px;
	}
	.nav-list.footer {
		margin-top: auto;
	}
	.nav-item {
		position: relative;
		display: flex;
		align-items: center;
		gap: 12px;
		height: 36px;
		padding: 0 12px;
		border-radius: var(--radius-control);
		color: var(--text-primary);
		text-decoration: none;
		cursor: default;
	}
	.nav-item:hover {
		background: var(--fill-subtle-hover);
	}
	.nav-item.active {
		background: var(--fill-subtle-hover);
	}
	.nav-item.active::before {
		content: '';
		position: absolute;
		left: 0;
		top: 10px;
		bottom: 10px;
		width: 3px;
		border-radius: 2px;
		background: var(--accent);
	}

	.content {
		overflow: auto;
		background: var(--layer);
		border: 1px solid var(--stroke-card);
		border-right: 0;
		border-top-left-radius: var(--radius-overlay);
	}

	.statusbar {
		display: flex;
		align-items: center;
		gap: 16px;
		height: 28px;
		padding: 0 12px;
		font-size: 12px;
		color: var(--text-secondary);
		background: var(--layer);
		border-left: 1px solid var(--stroke-card);
		border-top: 1px solid var(--stroke-divider);
	}
	.statusbar span {
		display: inline-flex;
		align-items: center;
		gap: 4px;
	}
	.statusbar .spacer {
		flex: 1;
	}
	.countdown {
		display: flex;
		align-items: center;
		gap: 12px;
		margin: 12px 20px 0;
		padding: 10px 14px;
		border-radius: var(--radius-control, 4px);
		color: var(--text-primary);
		background: color-mix(in srgb, var(--accent) 16%, var(--layer));
		border: 1px solid var(--accent);
	}
	.countdown span {
		flex: 1;
	}
	.statusbar .limit {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		height: 22px;
		padding: 0 8px;
		border: 0;
		border-radius: 4px;
		font: inherit;
		color: inherit;
		background: transparent;
		cursor: pointer;
	}
	.statusbar .limit:hover {
		background: var(--fill-subtle-hover);
	}
	.statusbar .limit.on {
		color: var(--accent-text-on-layer);
	}
	.statusbar .net {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		color: inherit;
		text-decoration: none;
	}
	.statusbar .net.down {
		color: var(--critical);
	}

	.badge {
		margin-left: auto;
		min-width: 20px;
		padding: 0 6px;
		border-radius: 10px;
		font-size: 11px;
		line-height: 18px;
		text-align: center;
		color: var(--accent-text);
		background: var(--accent);
	}

	.drop-overlay {
		position: fixed;
		inset: 0;
		z-index: 20;
		display: grid;
		place-items: center;
		background: color-mix(in srgb, var(--accent) 14%, transparent);
		border: 2px dashed var(--accent);
		pointer-events: none;
	}
	.drop-card {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 6px;
		padding: 24px 32px;
		border-radius: var(--radius-overlay);
		background: var(--page-fallback);
		color: var(--text-secondary);
		box-shadow: 0 8px 24px rgba(0, 0, 0, 0.18);
	}
	.drop-card strong {
		color: var(--text-primary);
		font: 600 16px/22px var(--font-display);
	}
</style>
