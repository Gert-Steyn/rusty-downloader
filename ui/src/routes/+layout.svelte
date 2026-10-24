<script lang="ts">
	import '#lib/styles/theme.css';
	import favicon from '#lib/assets/favicon.svg';
	import Icon from '#lib/components/Icon.svelte';
	import { mainNav, footerNav, type NavItem } from '#lib/nav.js';
	import { appInfo, isTauri } from '#lib/api.js';
	import { page } from '$app/state';
	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();

	let version = $state<string | null>(null);

	if (isTauri()) {
		document.documentElement.dataset.shell = 'tauri';
		appInfo()
			.then((info) => (version = info.version))
			.catch((e) => console.error('app_info failed', e));
	}

	const isActive = (item: NavItem) =>
		item.href === '/' ? page.url.pathname === '/' : page.url.pathname.startsWith(item.href);
</script>

<svelte:head>
	<link rel="icon" href={favicon} />
</svelte:head>

{#snippet navLink(item: NavItem)}
	<a href={item.href} class="nav-item" class:active={isActive(item)} aria-current={isActive(item) ? 'page' : undefined}>
		<Icon name={item.icon} size={18} />
		<span>{item.label}</span>
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
		{@render children()}
	</main>

	<footer class="statusbar">
		<span><Icon name="downloads" size={14} /> 0 B/s</span>
		<span>0 active</span>
		<span>0 queued</span>
		<span class="spacer"></span>
		<span>No speed limit</span>
	</footer>
</div>

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
</style>
