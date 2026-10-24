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
</script>

<span class="badge {value}" title={error ?? undefined}>
	{#if value === 'checking'}<span class="spinner" aria-hidden="true"></span>{:else}<span class="dot" aria-hidden="true"></span>{/if}
	{LABELS[value]}
</span>

<style>
	.badge {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		padding: 0 8px;
		height: 20px;
		border-radius: 10px;
		font-size: 12px;
		white-space: nowrap;
		color: var(--text-secondary);
		background: var(--fill-subtle-hover);
		--tone: var(--text-disabled);
	}
	.online {
		--tone: var(--success);
		color: var(--success);
	}
	.offline {
		--tone: var(--critical);
		color: var(--critical);
	}
	.error {
		--tone: var(--caution);
	}
	.checking {
		--tone: var(--accent);
	}
	.dot {
		width: 6px;
		height: 6px;
		border-radius: 50%;
		background: var(--tone);
	}
	.spinner {
		width: 10px;
		height: 10px;
		border: 2px solid var(--stroke-divider);
		border-top-color: var(--tone);
		border-radius: 50%;
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
