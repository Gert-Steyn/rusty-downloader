// The speed limit in force, for the status bar: the switch, a scheduled
// rule, or none. The backend reports changes (a rule starting or ending).
import { commands, events, isTauri, unwrap, type ActiveLimit } from './api.js';

class SpeedStore {
	active = $state<ActiveLimit>({ limit: null, source: 'none', until: null });
	error = $state<string | null>(null);
	#started = false;

	async start() {
		if (this.#started) return;
		this.#started = true;
		if (!isTauri()) return;
		await events.speedLimitStatus.listen((e) => (this.active = e.payload.active));
		this.active = await commands.speedLimitStatus();
	}

	/** Ask again, after settings changed here. */
	async refresh() {
		if (isTauri()) this.active = await commands.speedLimitStatus();
	}

	/** Flip the switch: limit now, or back to the schedule. */
	async toggle() {
		try {
			this.active = await unwrap(commands.setSpeedLimitOn(this.active.source !== 'manual'));
			this.error = null;
		} catch (e) {
			this.error = e instanceof Error ? e.message : String(e);
		}
	}
}

export const speed = new SpeedStore();
