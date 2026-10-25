<script lang="ts">
	import { open as pickFolder } from '@tauri-apps/plugin-dialog';
	import { commands, isTauri, unwrap, type AppInfo, type Settings } from '#lib/api.js';
	import { settings } from '#lib/settings.svelte.js';

	let info = $state<AppInfo | null>(null);
	let infoError = $state<string | null>(null);
	let flash = $state(false);

	$effect(() => {
		void settings.load();
		if (!isTauri()) {
			infoError = 'Running in a plain browser, so backend details are unavailable.';
			return;
		}
		unwrap(commands.appInfo())
			.then((i) => (info = i))
			.catch((e) => (infoError = String(e)));
	});

	// Brief "Saved" after each change.
	$effect(() => {
		if (!settings.saved) return;
		flash = true;
		const timer = setTimeout(() => (flash = false), 1500);
		return () => clearTimeout(timer);
	});

	const s = $derived(settings.value);

	function set<K extends keyof Settings>(key: K, value: Settings[K]) {
		void settings.update({ [key]: value } as Partial<Settings>);
	}

	/** Save a number; a rejected one (the error says why) goes back to the saved value. */
	async function number(key: 'maxActive' | 'maxConnections', input: HTMLInputElement) {
		const n = Number(input.value);
		if (Number.isInteger(n) && n !== s?.[key]) await settings.update({ [key]: n });
		if (settings.value) input.value = String(settings.value[key]);
	}

	async function chooseFolder() {
		const picked = await pickFolder({
			directory: true,
			defaultPath: s?.downloadDir ?? settings.downloadsFolder ?? undefined,
			title: 'Folder for new downloads'
		});
		if (typeof picked === 'string') set('downloadDir', picked);
	}

	function patterns(text: string) {
		set(
			'clipboardIgnore',
			text.split(/\r?\n/).map((l) => l.trim()).filter(Boolean)
		);
	}
</script>

<div class="page">
	<header>
		<h1>Settings</h1>
		<span class="status" role="status">
			{#if settings.error}<span class="error-text">{settings.error}</span>{:else if flash}Saved{/if}
		</span>
	</header>

	{#if !s}
		<p class="muted">{settings.error ?? 'Loading…'}</p>
	{:else}
		<section class="card" aria-labelledby="set-downloads">
			<h2 id="set-downloads">Downloads</h2>
			<div class="grid">
				<span class="label">Save new downloads to</span>
				<div class="control">
					<span class="path selectable">{s.downloadDir ?? settings.downloadsFolder ?? 'Downloads folder'}</span>
					<div class="row">
						<button class="btn" onclick={chooseFolder} disabled={!isTauri()}>Change…</button>
						{#if s.downloadDir}
							<button class="btn" onclick={() => set('downloadDir', null)}>Use the Downloads folder</button>
						{/if}
					</div>
					<small>Packages from the LinkGrabber and Add links go here unless you pick another folder for them.</small>
				</div>

				<label class="label" for="set-active">Downloads at once</label>
				<div class="control">
					<input
						id="set-active"
						class="textbox narrow"
						type="number"
						min="1"
						max="20"
						value={s.maxActive}
						onchange={(e) => number('maxActive', e.currentTarget)}
					/>
					<small>The rest wait in the queue and start in package order.</small>
				</div>

				<label class="label" for="set-connections">Connections per download</label>
				<div class="control">
					<input
						id="set-connections"
						class="textbox narrow"
						type="number"
						min="1"
						max="32"
						value={s.maxConnections}
						onchange={(e) => number('maxConnections', e.currentTarget)}
					/>
					<small>For plain files on servers that allow it. Applies to downloads that start after the change.</small>
				</div>
			</div>
		</section>

		<section class="card" aria-labelledby="set-grabber">
			<h2 id="set-grabber">LinkGrabber</h2>
			<label class="check">
				<input type="checkbox" checked={s.clipboardWatch} onchange={(e) => set('clipboardWatch', e.currentTarget.checked)} />
				<span>Watch the clipboard <small>Links you copy anywhere are picked up.</small></span>
			</label>
			<label class="check indent">
				<input
					type="checkbox"
					checked={s.clipboardAsk}
					disabled={!s.clipboardWatch}
					onchange={(e) => set('clipboardAsk', e.currentTarget.checked)}
				/>
				<span>Ask first <small>Show the copied links with an Add button instead of adding them.</small></span>
			</label>
			<label class="field indent">
				<span>Ignore copied links matching</span>
				<textarea
					class="textbox mono"
					rows="3"
					spellcheck="false"
					disabled={!s.clipboardWatch}
					placeholder={'*youtube.com/shorts/*\nexample.com'}
					value={s.clipboardIgnore.join('\n')}
					onchange={(e) => patterns(e.currentTarget.value)}
				></textarea>
				<small>One per line. <code>*</code> matches anything; a line without it matches anywhere in the link.</small>
			</label>
			<label class="check">
				<input type="checkbox" checked={s.warnDuplicates} onchange={(e) => set('warnDuplicates', e.currentTarget.checked)} />
				<span>
					Warn about links downloaded before
					<small>Marks them in the LinkGrabber, and unticks them while the file is still there.</small>
				</span>
			</label>
		</section>

		<section class="card" aria-labelledby="set-windows">
			<h2 id="set-windows">Windows</h2>
			<label class="check">
				<input
					type="checkbox"
					checked={s.startWithWindows}
					disabled={!isTauri()}
					onchange={(e) => set('startWithWindows', e.currentTarget.checked)}
				/>
				<span>Start with Windows</span>
			</label>
			<label class="check indent">
				<input
					type="checkbox"
					checked={s.startMinimized}
					disabled={!s.startWithWindows}
					onchange={(e) => set('startMinimized', e.currentTarget.checked)}
				/>
				<span>Start in the tray <small>Queued downloads carry on without opening the window.</small></span>
			</label>
			<label class="check">
				<input type="checkbox" checked={s.closeToTray} onchange={(e) => set('closeToTray', e.currentTarget.checked)} />
				<span>Keep running in the tray when the window is closed <small>Quit from the tray menu.</small></span>
			</label>
		</section>

		<section class="card" aria-labelledby="set-notify">
			<h2 id="set-notify">Notifications</h2>
			<p class="muted small">Shown while the app is in the background.</p>
			<label class="check">
				<input type="checkbox" checked={s.notifyFinished} onchange={(e) => set('notifyFinished', e.currentTarget.checked)} />
				<span>When a package finishes</span>
			</label>
			<label class="check">
				<input type="checkbox" checked={s.notifyFailed} onchange={(e) => set('notifyFailed', e.currentTarget.checked)} />
				<span>When a download fails</span>
			</label>
		</section>
	{/if}

	<section class="card" aria-labelledby="set-about">
		<h2 id="set-about">About</h2>
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
		{:else if infoError}
			<p class="muted">{infoError}</p>
		{:else}
			<p class="muted">Loading…</p>
		{/if}
	</section>
</div>

<style>
	.page {
		display: grid;
		gap: 12px;
		padding: 16px 20px;
		max-width: 900px;
	}
	header {
		display: flex;
		align-items: baseline;
		gap: 16px;
	}
	h1 {
		margin: 0 0 4px;
		font: 600 20px/28px var(--font-display);
	}
	.status {
		font-size: 13px;
		color: var(--text-secondary);
	}
	section {
		display: grid;
		gap: 10px;
		padding: 16px;
	}
	h2 {
		margin: 0 0 2px;
		font: 600 14px/20px var(--font-ui);
	}
	.grid {
		display: grid;
		grid-template-columns: 200px 1fr;
		align-items: start;
		gap: 14px 16px;
	}
	.label {
		padding-top: 6px;
		font-size: 13px;
	}
	.control,
	.check span {
		display: flex;
		flex-direction: column;
		gap: 4px;
		min-width: 0;
	}
	.row {
		display: flex;
		gap: 8px;
	}
	.narrow {
		width: 90px;
	}
	.check {
		display: flex;
		align-items: flex-start;
		gap: 10px;
		font-size: 13px;
	}
	.check input {
		margin-top: 2px;
	}
	.indent {
		margin-left: 26px;
	}
	small {
		font-size: 12px;
		color: var(--text-secondary);
	}
	code {
		font-family: var(--font-mono);
	}
	.mono {
		font-family: var(--font-mono);
		font-size: 12px;
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
	.small {
		font-size: 13px;
	}
</style>
