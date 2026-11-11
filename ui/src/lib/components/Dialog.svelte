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
		max-width: calc(100vw - 2 * var(--space-5));
		padding: 0;
		border: 1px solid var(--border);
		border-radius: var(--radius-xl);
		color: var(--text);
		background: var(--raised);
		box-shadow: var(--shadow-popover);
	}
	dialog::backdrop {
		background: var(--scrim);
	}
	.body {
		padding: var(--space-6) var(--space-6) 0;
		color: var(--text-secondary);
	}
	h2 {
		margin: 0 0 var(--space-3);
		font: var(--type-title);
		color: var(--text);
	}
	footer {
		display: flex;
		justify-content: flex-end;
		gap: var(--space-3);
		margin-top: var(--space-6);
		padding: 0 var(--space-6) var(--space-6);
	}
</style>
