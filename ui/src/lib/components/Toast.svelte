<script lang="ts">
	// Bottom-right notice for things that happen in the background
	// (links from the clipboard, downloads queued). Click to act on it.
	let {
		message,
		action,
		duration = 4000,
		onaction
	}: {
		message: { text: string; id: number } | null;
		action?: string;
		/** Milliseconds before it hides. */
		duration?: number;
		onaction?: () => void;
	} = $props();

	let visible = $state(false);

	$effect(() => {
		if (!message) return;
		message.id; // re-run for every new message, even with the same text
		visible = true;
		const timer = setTimeout(() => (visible = false), duration);
		return () => clearTimeout(timer);
	});
</script>

{#if visible && message}
	<div class="toast" role="status">
		<span>{message.text}</span>
		{#if action && onaction}
			<button
				class="btn ghost small"
				onclick={() => {
					visible = false;
					onaction();
				}}>{action}</button
			>
		{/if}
	</div>
{/if}

<style>
	.toast {
		position: fixed;
		right: var(--space-5);
		/* Sits just above the 28px status bar. */
		bottom: calc(28px + var(--space-5));
		z-index: 10;
		display: flex;
		align-items: center;
		gap: var(--space-4);
		max-width: 360px;
		padding: var(--space-4);
		font: var(--type-body);
		color: var(--text);
		background: var(--raised);
		border: 1px solid var(--border);
		border-radius: var(--radius-lg);
		box-shadow: var(--shadow-popover);
		animation: rise var(--motion-base) var(--ease);
	}
	@keyframes rise {
		from {
			transform: translateY(8px);
			opacity: 0;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.toast {
			animation: none;
		}
	}
</style>
