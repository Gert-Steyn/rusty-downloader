<script lang="ts">
	import { open as pickFolder } from '@tauri-apps/plugin-dialog';
	import { commands, isTauri, unwrap, type AppInfo, type Settings, type SpeedRule } from '#lib/api.js';
	import AccountsSettings from '#lib/components/AccountsSettings.svelte';
	import KnownHostsSettings from '#lib/components/KnownHostsSettings.svelte';
	import NetworkSettings from '#lib/components/NetworkSettings.svelte';
	import { settings } from '#lib/settings.svelte.js';
	import { speed } from '#lib/speed.svelte.js';
	import { loadTheme, saveTheme, type ThemeChoice } from '#lib/theme.js';

	const THEMES: { value: ThemeChoice; label: string }[] = [
		{ value: 'system', label: 'System' },
		{ value: 'dark', label: 'Dark' },
		{ value: 'light', label: 'Light' }
	];
	let theme = $state<ThemeChoice>(loadTheme());

	function chooseTheme(choice: ThemeChoice) {
		theme = choice;
		saveTheme(choice);
	}

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

	const MB = 1024 * 1024;
	const DAYS = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday'];

	/** MB/s for a limit field, without trailing zeros. */
	const mbps = (bytes: number) => String(Math.round((bytes / MB) * 100) / 100);

	/** Bytes per second from a MB/s field, or null if it isn't a positive number. */
	function fromMbps(input: HTMLInputElement): number | null {
		const n = Number(input.value.replace(',', '.'));
		return Number.isFinite(n) && n > 0 ? Math.round(n * MB) : null;
	}

	async function setSpeedLimit(input: HTMLInputElement) {
		const bytes = fromMbps(input);
		if (bytes != null && bytes !== s?.speedLimit) await settings.update({ speedLimit: bytes });
		if (settings.value) input.value = mbps(settings.value.speedLimit);
		await speed.refresh();
	}

	async function setSwitch(on: boolean) {
		await settings.update({ speedLimitOn: on });
		await speed.refresh();
	}

	/** Save the schedule with `rule` at `i` changed (or removed, or added at the end). */
	async function saveRule(i: number, rule: SpeedRule | null) {
		const rules = [...(s?.speedSchedule ?? [])];
		if (rule == null) rules.splice(i, 1);
		else rules[i] = rule;
		await settings.update({ speedSchedule: rules });
		await speed.refresh();
	}

	function toggleDay(i: number, rule: SpeedRule, day: number) {
		const days = rule.days.includes(day) ? rule.days.filter((d) => d !== day) : [...rule.days, day];
		void saveRule(i, { ...rule, days });
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

	<section aria-labelledby="set-appearance">
		<h2 id="set-appearance">Appearance</h2>
		<div class="setting">
			<span class="label" id="set-theme">Theme</span>
			<div class="segmented" role="group" aria-labelledby="set-theme">
				{#each THEMES as t (t.value)}
					<button type="button" class="segment" aria-pressed={theme === t.value} onclick={() => chooseTheme(t.value)}>
						{t.label}
					</button>
				{/each}
			</div>
		</div>
		<small>System follows the Windows setting. Dark or Light applies on this device straight away.</small>
	</section>

	{#if !s}
		<p class="muted">{settings.error ?? 'Loading…'}</p>
	{:else}
		<section aria-labelledby="set-downloads">
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

		<section aria-labelledby="set-speed">
			<h2 id="set-speed">Speed</h2>
			<label class="check">
				<input type="checkbox" checked={s.speedLimitOn} onchange={(e) => setSwitch(e.currentTarget.checked)} />
				<span>
					<span class="inline">
						Limit downloads to
						<input
							class="textbox narrow"
							type="number"
							min="0.01"
							step="0.1"
							aria-label="Speed limit in MB/s"
							value={mbps(s.speedLimit)}
							onchange={(e) => setSpeedLimit(e.currentTarget)}
						/>
						MB/s
					</span>
					<small>The same switch is in the status bar. yt-dlp and gallery-dl downloads take the new limit when they next start.</small>
				</span>
			</label>

			<h3>Scheduled limits</h3>
			<small>Apply by themselves at these times while the switch above is off; the lowest one counts. A window that ends before it starts runs overnight.</small>
			{#each s.speedSchedule as rule, i (i)}
				<div class="rule">
					<div class="days" role="group" aria-label="Days">
						{#each DAYS as day, d (day)}
							<button
								type="button"
								class="day"
								class:on={rule.days.includes(d)}
								aria-pressed={rule.days.includes(d)}
								aria-label={day}
								title={day}
								onclick={() => toggleDay(i, rule, d)}>{day[0]}</button
							>
						{/each}
					</div>
					<input
						class="textbox"
						type="time"
						aria-label="From"
						value={rule.from}
						onchange={(e) => saveRule(i, { ...rule, from: e.currentTarget.value })}
					/>
					<span class="muted">to</span>
					<input
						class="textbox"
						type="time"
						aria-label="To"
						value={rule.to}
						onchange={(e) => saveRule(i, { ...rule, to: e.currentTarget.value })}
					/>
					<input
						class="textbox narrow"
						type="number"
						min="0.01"
						step="0.1"
						aria-label="Limit in MB/s"
						value={mbps(rule.limit)}
						onchange={(e) => {
							const limit = fromMbps(e.currentTarget);
							if (limit != null) saveRule(i, { ...rule, limit });
							else e.currentTarget.value = mbps(rule.limit);
						}}
					/>
					<span class="muted">MB/s</span>
					<button type="button" class="btn" aria-label="Remove this limit" onclick={() => saveRule(i, null)}>Remove</button>
				</div>
			{/each}
			<div class="row">
				<button
					type="button"
					class="btn"
					onclick={() =>
						saveRule(s?.speedSchedule.length ?? 0, { days: [0, 1, 2, 3, 4], from: '08:00', to: '18:00', limit: MB })}
					>Add a scheduled limit</button
				>
			</div>
		</section>

		<NetworkSettings />

		<AccountsSettings />

		<KnownHostsSettings />

		<section aria-labelledby="set-grabber">
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

		<section aria-labelledby="set-windows">
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
			<label class="check">
				<input type="checkbox" checked={s.keepAwake} onchange={(e) => set('keepAwake', e.currentTarget.checked)} />
				<span>Keep the computer awake while downloading <small>Windows won’t go to sleep while downloads run; the screen can still turn off.</small></span>
			</label>
		</section>

		<section aria-labelledby="set-notify">
			<h2 id="set-notify">Notifications</h2>
			<p class="muted">Shown while the app is in the background.</p>
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

	<section aria-labelledby="set-about">
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
		gap: var(--space-6);
		max-width: 900px;
		padding: var(--page-pad);
	}
	header {
		display: flex;
		align-items: baseline;
		gap: var(--space-5);
	}
	h1 {
		margin: 0;
		font: var(--type-title);
		color: var(--text);
	}
	.status {
		font: var(--type-small);
		color: var(--text-muted);
	}
	/* Groups: a heading, then rows divided by hairlines. No card chrome. */
	section {
		display: grid;
	}
	h2 {
		margin: 0 0 var(--space-3);
		font: var(--type-heading);
		color: var(--text);
	}
	.grid,
	.setting {
		display: grid;
		grid-template-columns: 200px 1fr;
		column-gap: 0;
	}
	.grid {
		align-items: start;
	}
	.setting {
		align-items: center;
	}
	.grid > * {
		padding: var(--space-4) 0;
		border-top: 1px solid var(--border-subtle);
	}
	.setting {
		padding: var(--space-4) 0;
		border-top: 1px solid var(--border-subtle);
	}
	.label {
		padding-right: var(--space-5);
		font: var(--type-label);
		color: var(--text);
	}
	.control,
	.check span {
		display: flex;
		flex-direction: column;
		gap: var(--space-2);
		min-width: 0;
	}
	.row {
		display: flex;
		flex-wrap: wrap;
		gap: var(--space-3);
	}
	.narrow {
		width: 90px;
	}
	h3 {
		margin: var(--space-4) 0 0;
		font: var(--type-label);
		color: var(--text);
	}
	.inline {
		display: flex !important;
		flex-direction: row !important;
		align-items: center;
		gap: var(--space-3);
	}
	.rule {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--space-3);
		padding: var(--space-4) 0;
		border-top: 1px solid var(--border-subtle);
		font: var(--type-body);
	}
	.rule .textbox {
		width: 90px;
	}
	.rule input[type='time'] {
		width: 120px;
	}
	.days {
		display: flex;
		gap: var(--space-1);
	}
	.day {
		width: var(--control-h);
		height: var(--control-h);
		padding: 0;
		font: var(--type-label);
		color: var(--text-secondary);
		background: transparent;
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		cursor: default;
		transition:
			background var(--motion-fast) var(--ease),
			border-color var(--motion-fast) var(--ease),
			color var(--motion-fast) var(--ease);
	}
	.day:hover {
		color: var(--text);
		border-color: var(--border-strong);
	}
	.day:active {
		background: var(--pressed);
	}
	.day[aria-pressed='true'] {
		color: var(--on-accent);
		background: var(--accent-fill);
		border-color: transparent;
	}
	.day[aria-pressed='true']:hover {
		background: var(--accent-fill-hover);
	}
	.day[aria-pressed='true']:active {
		background: var(--accent-fill-pressed);
	}
	.check {
		display: flex;
		align-items: flex-start;
		gap: var(--space-3);
		padding: var(--space-4) 0;
		border-top: 1px solid var(--border-subtle);
		font: var(--type-body);
	}
	.check input {
		margin-top: var(--space-1);
	}
	.indent {
		margin-left: var(--space-6);
	}
	.field {
		margin: 0;
		padding: var(--space-4) 0;
		border-top: 1px solid var(--border-subtle);
	}
	small {
		font: var(--type-small);
		color: var(--text-muted);
	}
	code {
		font: var(--type-mono);
	}
	/* Segmented control: mode switch within one view. */
	.segmented {
		display: inline-flex;
		justify-self: start;
		gap: var(--space-1);
		padding: var(--space-1);
		background: var(--surface);
		border: 1px solid var(--border-subtle);
		border-radius: var(--radius-md);
	}
	.segment {
		height: var(--control-h-sm);
		padding: 0 var(--space-3);
		font: var(--type-label);
		color: var(--text-secondary);
		background: transparent;
		border: 0;
		border-radius: var(--radius-sm);
		cursor: default;
		transition:
			background var(--motion-fast) var(--ease),
			color var(--motion-fast) var(--ease);
	}
	.segment:hover {
		color: var(--text);
	}
	.segment:active {
		background: var(--pressed);
	}
	.segment[aria-pressed='true'] {
		color: var(--text);
		background: var(--hover);
	}
	dl {
		display: grid;
		grid-template-columns: 120px 1fr;
		gap: var(--space-3) var(--space-5);
		margin: 0;
		font: var(--type-body);
	}
	dt {
		color: var(--text-secondary);
	}
	dd {
		margin: 0;
	}
	.path {
		font: var(--type-mono);
		word-break: break-all;
	}
</style>
