<script lang="ts">
	import { appInfo, isTauri, type AppInfo } from '#lib/api.js';

	let info = $state<AppInfo | null>(null);
	let error = $state<string | null>(null);

	$effect(() => {
		if (!isTauri()) {
			error = 'Running in a plain browser, so backend details are unavailable.';
			return;
		}
		appInfo()
			.then((i) => (info = i))
			.catch((e) => (error = String(e)));
	});
</script>

<div class="page">
	<h1>Settings</h1>

	<section class="card">
		<h2>About</h2>
		{#if info}
			<dl>
				<dt>Version</dt>
				<dd>{info.version}</dd>
				<dt>Mode</dt>
				<dd>{info.portable ? 'Portable' : 'Installed'}</dd>
				<dt>Data folder</dt>
				<dd class="path selectable">{info.dataDir}</dd>
				<dt>Database</dt>
				<dd class="path selectable">{info.dbFile} (schema v{info.schemaVersion})</dd>
				<dt>Logs</dt>
				<dd class="path selectable">{info.logsDir}</dd>
			</dl>
		{:else if error}
			<p class="muted">{error}</p>
		{:else}
			<p class="muted">Loading…</p>
		{/if}
	</section>
</div>

<style>
	.page {
		padding: 16px 20px;
		max-width: 900px;
	}
	h1 {
		margin: 0 0 16px;
		font: 600 20px/28px var(--font-display);
	}
	section {
		padding: 16px;
	}
	h2 {
		margin: 0 0 12px;
		font: 600 14px/20px var(--font-ui);
	}
	dl {
		display: grid;
		grid-template-columns: 120px 1fr;
		gap: 8px 16px;
		margin: 0;
	}
	dt {
		color: var(--text-secondary);
	}
	dd {
		margin: 0;
	}
	.path {
		font-family: var(--font-mono);
		font-size: 12px;
		word-break: break-all;
	}
	.muted {
		margin: 0;
		color: var(--text-secondary);
	}
</style>
