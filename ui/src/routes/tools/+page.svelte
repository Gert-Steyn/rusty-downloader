<script lang="ts">
	import Icon from '#lib/components/Icon.svelte';
	import { isTauri, type ToolInfo, type ToolName } from '#lib/api.js';
	import { formatBytes, formatDate } from '#lib/format.js';
	import { tools } from '#lib/tools.svelte.js';

	const ABOUT: Record<ToolName, { title: string; body: string; size: string }> = {
		ytdlp: {
			title: 'yt-dlp',
			body: 'Downloads video and audio from YouTube and thousands of other sites. Updated automatically once a day, because sites change often.',
			size: 'about 18 MB'
		},
		ffmpeg: {
			title: 'ffmpeg',
			body: 'Merges separate video and audio streams and converts audio. Installed the first time a video download needs it; updated only when you ask.',
			size: 'about 90 MB'
		}
	};

	$effect(() => {
		void tools.start();
	});

	function busy(tool: ToolInfo) {
		return tool.task != null;
	}

	function taskText(tool: ToolInfo): string {
		const task = tool.task;
		if (!task) return '';
		switch (task.type) {
			case 'checking':
				return 'Checking for updates…';
			case 'installing':
				return 'Installing…';
			case 'downloading':
				return task.total
					? `Downloading ${formatBytes(task.downloaded)} of ${formatBytes(task.total)}`
					: `Downloading ${formatBytes(task.downloaded)}`;
		}
	}

	function taskPercent(tool: ToolInfo): number | null {
		const task = tool.task;
		if (task?.type !== 'downloading' || !task.total) return null;
		return Math.min(100, (task.downloaded / task.total) * 100);
	}
</script>

<div class="page">
	<header class="toolbar">
		<h1>Tools</h1>
		<span class="spacer"></span>
		<button class="btn" disabled={!isTauri()} onclick={tools.openFolder}><Icon name="folder" /> Open tools folder</button>
	</header>

	{#if tools.error}
		<p class="banner" role="alert">{tools.error}</p>
	{/if}

	<div class="cards">
		{#each tools.tools as tool (tool.name)}
			{@const about = ABOUT[tool.name]}
			{@const installed = tool.path != null}
			<section class="card tool" aria-labelledby="tool-{tool.name}">
				<div class="head">
					<div>
						<h2 id="tool-{tool.name}">{about.title}</h2>
						<p class="muted">{about.body}</p>
					</div>
					<span class="state" class:ok={installed && !tool.updateAvailable} class:warn={tool.updateAvailable}>
						{#if !installed}Not installed{:else if tool.updateAvailable}Update available{:else}Up to date{/if}
					</span>
				</div>

				<dl>
					<dt>Version</dt>
					<dd class="mono selectable">{tool.version ?? '—'}</dd>
					<dt>Release</dt>
					<dd>
						{tool.release ?? '—'}
						{#if tool.installedAt}<span class="muted"> · installed {formatDate(tool.installedAt)}</span>{/if}
					</dd>
					<dt>Newest</dt>
					<dd>
						{tool.latestRelease ?? 'Not checked yet'}
						{#if tool.lastCheck}<span class="muted"> · checked {formatDate(tool.lastCheck)}</span>{/if}
					</dd>
					{#if tool.name === 'ytdlp'}
						<dt>Channel</dt>
						<dd>
							<select
								class="select"
								aria-label="yt-dlp release channel"
								value={tool.channel}
								disabled={busy(tool)}
								onchange={(e) => tools.setChannel(tool.name, e.currentTarget.value as 'stable' | 'nightly')}
							>
								<option value="stable">Stable</option>
								<option value="nightly">Nightly (fixes for broken sites land here first)</option>
							</select>
						</dd>
					{/if}
					{#if tool.path}
						<dt>Location</dt>
						<dd class="mono selectable path">{tool.path}</dd>
					{/if}
				</dl>

				{#if tool.task}
					{@const pct = taskPercent(tool)}
					<div class="task" role="status">
						<span>{taskText(tool)}</span>
						<div class="track" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow={pct ?? undefined}>
							{#if pct == null}<div class="indeterminate"></div>{:else}<div class="fill" style:width="{pct}%"></div>{/if}
						</div>
					</div>
				{:else if tool.error}
					<p class="error" role="alert">{tool.error}</p>
				{/if}

				<div class="actions">
					{#if !installed}
						<button class="btn accent" disabled={busy(tool) || !isTauri()} onclick={() => tools.install(tool.name)}>
							<Icon name="downloads" /> Install ({about.size})
						</button>
					{:else if tool.updateAvailable}
						<button class="btn accent" disabled={busy(tool)} onclick={() => tools.install(tool.name)}>
							<Icon name="downloads" /> Update to {tool.latestRelease}
						</button>
					{/if}
					<button class="btn" disabled={busy(tool) || !isTauri()} onclick={() => tools.check(tool.name)}>
						<Icon name="refresh" /> Check for updates
					</button>
					{#if tool.previousVersion}
						<button
							class="btn"
							disabled={busy(tool)}
							title="Switch back to the version used before the last update"
							onclick={() => tools.rollback(tool.name)}
						>
							<Icon name="undo" /> Roll back to {tool.previousVersion}
						</button>
					{/if}
				</div>
			</section>
		{/each}
	</div>
	<p class="footnote muted">
		Tools are downloaded from their official GitHub releases and checked against the published SHA-256 checksums
		before use. The previous version is kept, so a bad update can be rolled back.
	</p>
</div>

<style>
	.page {
		padding: 16px 20px;
		max-width: 900px;
	}
	.toolbar {
		display: flex;
		align-items: center;
		gap: 8px;
		margin-bottom: 16px;
	}
	h1 {
		margin: 0;
		font: 600 20px/28px var(--font-display);
	}
	.spacer {
		flex: 1;
	}
	.banner {
		margin: 0 0 12px;
		padding: 8px 16px;
		border-radius: var(--radius-control);
		background: color-mix(in srgb, var(--critical) 12%, transparent);
	}
	.cards {
		display: grid;
		gap: 12px;
	}
	.tool {
		padding: 16px;
	}
	.head {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: 16px;
		margin-bottom: 12px;
	}
	h2 {
		margin: 0 0 4px;
		font: 600 16px/22px var(--font-ui);
	}
	.muted {
		margin: 0;
		color: var(--text-secondary);
	}
	.state {
		flex-shrink: 0;
		padding: 2px 10px;
		border-radius: 10px;
		font-size: 12px;
		color: var(--text-secondary);
		background: var(--fill-subtle-hover);
	}
	.state.ok {
		color: var(--success);
		background: color-mix(in srgb, var(--success) 12%, transparent);
	}
	.state.warn {
		color: var(--caution);
		background: color-mix(in srgb, var(--caution) 14%, transparent);
	}
	dl {
		display: grid;
		grid-template-columns: 100px 1fr;
		gap: 6px 16px;
		margin: 0 0 12px;
		font-size: 13px;
	}
	dt {
		color: var(--text-secondary);
	}
	dd {
		margin: 0;
		min-width: 0;
	}
	.mono {
		font-family: var(--font-mono);
		font-size: 12px;
	}
	.path {
		word-break: break-all;
	}
	.task {
		display: grid;
		gap: 6px;
		margin-bottom: 12px;
		font-size: 13px;
	}
	.track {
		position: relative;
		height: 4px;
		overflow: hidden;
		border-radius: 2px;
		background: var(--stroke-divider);
	}
	.fill {
		height: 100%;
		border-radius: 2px;
		background: var(--accent);
		transition: width 0.2s;
	}
	.indeterminate {
		position: absolute;
		inset: 0 auto 0 0;
		width: 30%;
		border-radius: 2px;
		background: var(--accent);
		animation: slide 1.2s ease-in-out infinite;
	}
	@keyframes slide {
		from {
			left: -30%;
		}
		to {
			left: 100%;
		}
	}
	.error {
		margin: 0 0 12px;
		font-size: 13px;
		color: var(--critical);
	}
	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
	}
	.footnote {
		margin-top: 16px;
		font-size: 12px;
	}
</style>
