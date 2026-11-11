<script lang="ts">
	import type { Availability } from '#lib/api.js';

	let { value, error = null }: { value: Availability; error?: string | null } = $props();

	const LABELS: Record<Availability, string> = {
		unknown: 'Waiting',
		checking: 'Checking',
		online: 'Online',
		offline: 'Offline',
		error: 'Check failed'
	};

	// Semantic colour only where it carries meaning; waiting stays neutral.
	const VARIANT: Record<Availability, '' | 'accent' | 'success' | 'warning' | 'danger'> = {
		unknown: '',
		checking: 'accent',
		online: 'success',
		offline: 'danger',
		error: 'warning'
	};
</script>

<span class="tag {VARIANT[value]}" title={error ?? undefined}>
	{#if value === 'checking'}<span class="spinner" aria-hidden="true"></span>{:else}<span class="dot" aria-hidden="true"></span>{/if}
	{LABELS[value]}
</span>

<style>
	.dot {
		width: 8px;
		height: 8px;
		border-radius: var(--radius-full);
		background: currentColor;
	}
	.spinner {
		width: 10px;
		height: 10px;
		border: 2px solid var(--border);
		border-top-color: currentColor;
		border-radius: var(--radius-full);
		animation: spin 0.8s linear infinite;
	}
	@keyframes spin {
		to {
			transform: rotate(360deg);
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.spinner {
			animation: none;
		}
	}
</style>
