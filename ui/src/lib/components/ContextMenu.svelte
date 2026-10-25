<script lang="ts" module>
	import type { IconName } from './Icon.svelte';

	export type MenuItem =
		| { label: string; icon?: IconName; disabled?: boolean; danger?: boolean; action: () => void }
		| 'separator';
</script>

<script lang="ts">
	// Right-click menu at the pointer. Closes on a pick, Escape, a click
	// elsewhere, scrolling or resizing. Arrow keys move between items.
	import Icon from './Icon.svelte';

	let {
		at = $bindable(null),
		items
	}: {
		/** Where to open (client coordinates); `null` closes it. */
		at?: { x: number; y: number } | null;
		items: MenuItem[];
	} = $props();

	let menu = $state<HTMLDivElement>();
	let pos = $state({ x: 0, y: 0 });

	// Keep the menu on screen: flip left/up when it would overflow.
	$effect(() => {
		if (!at || !menu) return;
		const { width, height } = menu.getBoundingClientRect();
		pos = {
			x: at.x + width > innerWidth - 4 ? Math.max(4, at.x - width) : at.x,
			y: at.y + height > innerHeight - 4 ? Math.max(4, at.y - height) : at.y
		};
		menu.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus();
	});

	function close() {
		at = null;
	}

	function pick(item: Exclude<MenuItem, 'separator'>) {
		close();
		item.action();
	}

	function onkeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			close();
			e.preventDefault();
			return;
		}
		if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
		e.preventDefault();
		const buttons = [...(menu?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? [])];
		const i = buttons.indexOf(document.activeElement as HTMLButtonElement);
		const next = e.key === 'ArrowDown' ? (i + 1) % buttons.length : (i - 1 + buttons.length) % buttons.length;
		buttons[next]?.focus();
	}
</script>

<svelte:window
	onpointerdown={(e) => at && !menu?.contains(e.target as Node) && close()}
	onblur={() => at && close()}
	onresize={() => at && close()}
	onwheel={() => at && close()}
/>

{#if at}
	<div
		bind:this={menu}
		class="menu"
		role="menu"
		tabindex="-1"
		style:left="{pos.x}px"
		style:top="{pos.y}px"
		{onkeydown}
		oncontextmenu={(e) => e.preventDefault()}
	>
		{#each items as item, i (i)}
			{#if item === 'separator'}
				<div class="sep" role="separator"></div>
			{:else}
				<button role="menuitem" class:danger={item.danger} disabled={item.disabled} onclick={() => pick(item)}>
					<span class="icon">{#if item.icon}<Icon name={item.icon} size={16} />{/if}</span>
					{item.label}
				</button>
			{/if}
		{/each}
	</div>
{/if}

<style>
	.menu {
		position: fixed;
		z-index: 30;
		min-width: 200px;
		padding: 4px;
		border: 1px solid var(--stroke-card);
		border-radius: var(--radius-overlay);
		background: var(--page-fallback);
		box-shadow: 0 8px 16px rgba(0, 0, 0, 0.14);
		outline: none;
	}
	button {
		display: flex;
		align-items: center;
		gap: 10px;
		width: 100%;
		height: 32px;
		padding: 0 12px 0 8px;
		border: 0;
		border-radius: var(--radius-control);
		font: inherit;
		font-size: 13px;
		text-align: left;
		color: var(--text-primary);
		background: transparent;
		cursor: default;
	}
	button:hover:not(:disabled),
	button:focus-visible {
		background: var(--fill-subtle-hover);
		outline: none;
	}
	button:disabled {
		color: var(--text-secondary);
		opacity: 0.6;
	}
	button.danger {
		color: var(--critical);
	}
	.icon {
		display: inline-flex;
		width: 16px;
	}
	.sep {
		height: 1px;
		margin: 4px 0;
		background: var(--stroke-divider);
	}
</style>
