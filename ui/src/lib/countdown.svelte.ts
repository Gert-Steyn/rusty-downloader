// A queue's "when finished" countdown (sleep, shut down, ...), shown as a
// banner with Cancel. The backend counts; this ticks the seconds shown.
import { commands, events, isTauri, type PendingAction } from './api.js';

class CountdownStore {
	pending = $state<PendingAction | null>(null);
	secondsLeft = $state(0);
	#deadline = 0;
	#timer: ReturnType<typeof setInterval> | undefined;
	#started = false;

	async start() {
		if (this.#started || !isTauri()) return;
		this.#started = true;
		await events.onCompleteCountdown.listen((e) => this.#show(e.payload.pending));
		this.#show(await commands.onCompletePending());
	}

	#show(pending: PendingAction | null) {
		clearInterval(this.#timer);
		this.pending = pending;
		if (!pending) return;
		this.#deadline = Date.now() + pending.secondsLeft * 1000;
		const tick = () => (this.secondsLeft = Math.max(0, Math.ceil((this.#deadline - Date.now()) / 1000)));
		tick();
		this.#timer = setInterval(tick, 250);
	}

	async cancel() {
		await commands.cancelOnComplete();
		this.#show(null);
	}
}

export const countdown = new CountdownStore();

export const ACTION_LABEL: Record<PendingAction['action'], string> = {
	none: 'Do nothing',
	sleep: 'Sleep',
	hibernate: 'Hibernate',
	shutdown: 'Shut down',
	exit: 'Close Rusty Downloader'
};

/** "Shutting down in 42 s" */
export function countdownText(action: PendingAction['action'], seconds: number): string {
	const doing: Record<PendingAction['action'], string> = {
		none: 'Doing nothing',
		sleep: 'Going to sleep',
		hibernate: 'Hibernating',
		shutdown: 'Shutting down',
		exit: 'Closing Rusty Downloader'
	};
	return `${doing[action]} in ${seconds} s`;
}
