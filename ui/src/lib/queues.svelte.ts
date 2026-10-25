// Queues for the Queues page and the package menu's "Move to" entries.
// Re-read whenever the backend says queues or their contents changed.
import { commands, events, isTauri, unwrap, type Queue, type QueueChanges } from './api.js';

class QueuesStore {
	list = $state<Queue[]>([]);
	error = $state<string | null>(null);
	#started = false;

	async start() {
		if (this.#started) return;
		this.#started = true;
		if (!isTauri()) {
			if (import.meta.env.DEV) this.list = (await import('./mock.js')).queues();
			return;
		}
		await events.downloadEvents.listen((e) => {
			// Counts change with every status change; queue settings with QueuesChanged.
			if (e.payload.resync || e.payload.events.some((ev) => ev.type !== 'progress')) void this.refresh();
		});
		await this.refresh();
	}

	async refresh() {
		try {
			this.list = await unwrap(commands.listQueues());
			this.error = null;
		} catch (e) {
			this.error = String(e);
		}
	}

	async run(action: () => Promise<unknown>): Promise<boolean> {
		try {
			await action();
			this.error = null;
			await this.refresh();
			return true;
		} catch (e) {
			this.error = e instanceof Error ? e.message : String(e);
			await this.refresh();
			return false;
		}
	}

	add = (name: string) => this.run(() => unwrap(commands.addQueue(name)));
	update = (id: number, changes: QueueChanges) => this.run(() => unwrap(commands.updateQueue(id, changes)));
	remove = (id: number) => this.run(() => unwrap(commands.removeQueue(id)));
	movePackage = (packageId: number, queue: number) =>
		this.run(() => unwrap(commands.movePackageToQueue(packageId, queue)));
}

export const queues = new QueuesStore();
