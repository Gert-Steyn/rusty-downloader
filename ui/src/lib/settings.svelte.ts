// The app's preferences. Every change is saved right away; the backend
// checks values and returns them as stored.
import { commands, isTauri, unwrap, type Settings } from './api.js';
import { grabber } from './linkgrabber.svelte.js';

class SettingsStore {
	value = $state<Settings | null>(null);
	/** The folder new packages go to when none is chosen (the user's Downloads). */
	downloadsFolder = $state<string | null>(null);
	error = $state<string | null>(null);
	/** Bumped on every successful save, for a "Saved" flash. */
	saved = $state(0);

	/**
	 * Read the settings fresh, each time the page opens: some are also changed
	 * elsewhere (the LinkGrabber's clipboard switch, Windows' startup list),
	 * and saving sends them all back.
	 */
	async load() {
		if (!isTauri()) {
			const mock = await import('./mock.js');
			this.value = mock.settings();
			this.downloadsFolder = 'C:\\Users\\You\\Downloads';
			return;
		}
		try {
			this.value = await commands.getSettings();
			this.downloadsFolder = await unwrap(commands.systemDownloadsDir());
		} catch (e) {
			this.error = e instanceof Error ? e.message : String(e);
		}
	}

	async update(patch: Partial<Settings>) {
		if (!this.value) return;
		const next = { ...this.value, ...patch };
		if (!isTauri()) {
			this.value = next;
			this.saved++;
			return;
		}
		try {
			this.value = await unwrap(commands.setSettings(next));
			this.error = null;
			grabber.clipboardWatch = this.value.clipboardWatch;
			this.saved++;
		} catch (e) {
			this.error = e instanceof Error ? e.message : String(e);
		}
	}
}

export const settings = new SettingsStore();
