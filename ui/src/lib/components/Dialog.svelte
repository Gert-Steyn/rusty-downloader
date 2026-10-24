<script lang="ts">
	// Modal built on <dialog>: the browser handles focus trapping, Esc and the backdrop.
	import type { Snippet } from 'svelte';

	let {
		open = $bindable(false),
		title,
		width = 520,
		children,
		footer,
		onsubmit
	}: {
		open?: boolean;
		title: string;
		width?: number;
		children: Snippet;
		footer: Snippet;
		onsubmit?: () => void;
	} = $props();

	let dialog: HTMLDialogElement;

	$effect(() => {
		if (open && !dialog.open) dialog.showModal();
		if (!open && dialog.open) dialog.close();
	});
</script>

<dialog bind:this={dialog} onclose={() => (open = false)} style:width="{width}px" aria-labelledby="dialog-title">
	<form
		method="dialog"
		onsubmit={(e) => {
			if (onsubmit) {
				e.preventDefault();
				onsubmit();
			}
		}}
	>
		<div class="body">
			<h2 id="dialog-title">{title}</h2>
			{@render children()}
		</div>
		<footer>{@render footer()}</footer>
	</form>
</dialog>

<style>
	dialog {
		max-width: calc(100vw - 32px);
		padding: 0;
		border: 1px solid var(--stroke-card);
		border-radius: var(--radius-overlay);
		color: var(--text-primary);
		background: var(--page-fallback);
		box-shadow: 0 32px 64px rgba(0, 0, 0, 0.19), 0 2px 21px rgba(0, 0, 0, 0.15);
	}
	dialog::backdrop {
		background: rgba(0, 0, 0, 0.3);
	}
	.body {
		padding: 24px;
	}
	h2 {
		margin: 0 0 16px;
		font: 600 20px/28px var(--font-display);
	}
	footer {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
		padding: 20px 24px;
		background: var(--layer);
		border-top: 1px solid var(--stroke-divider);
	}
</style>
