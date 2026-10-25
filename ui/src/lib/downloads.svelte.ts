// Live view of the download manager: the package list from `listPackages`,
// kept current by the batched `downloadEvents` stream.
import {
	commands,
	events,
	isTauri,
	unwrap,
	type Download,
	type DownloadEventBatch,
	type Package
} from './api.js';

export interface Live {
	speedBps: number;
	connections: number;
}

class DownloadsStore {
	packages = $state<Package[]>([]);
	/** Speed and connections, only for downloads that are running. */
	live = $state<Record<number, Live>>({});
	ready = $state(false);
	error = $state<string | null>(null);

	stats = $derived.by(() => {
		let speed = 0;
		let active = 0;
		let queued = 0;
		for (const p of this.packages) {
			for (const d of p.downloads) {
				if (d.status === 'active') {
					active++;
					speed += this.live[d.id]?.speedBps ?? 0;
				} else if (d.status === 'queued') {
					queued++;
				}
			}
		}
		return { speed, active, queued };
	});

	#byId = new Map<number, Download>();
	#started = false;

	async start() {
		if (this.#started) return;
		this.#started = true;
		if (!isTauri()) {
			// Plain-browser preview during UI development: show sample data.
			if (import.meta.env.DEV) {
				const mock = await import('./mock.js');
				this.packages = mock.packages();
				this.live = mock.live();
				this.#reindex();
			}
			this.ready = true;
			return;
		}
		await events.downloadEvents.listen((e) => this.#apply(e.payload));
		await this.refresh();
	}

	async refresh() {
		try {
			this.packages = await unwrap(commands.listPackages());
			this.#reindex();
			this.error = null;
		} catch (e) {
			this.error = String(e);
		} finally {
			this.ready = true;
		}
	}

	/** Reactive lookup (reads `packages`), for components. Event handling uses the index. */
	download(id: number): Download | undefined {
		for (const p of this.packages) {
			const d = p.downloads.find((d) => d.id === id);
			if (d) return d;
		}
		return undefined;
	}

	package(id: number): Package | undefined {
		return this.packages.find((p) => p.id === id);
	}

	/** Run a command, surfacing failures in `error` instead of throwing. */
	async run<T>(action: () => Promise<T>): Promise<T | undefined> {
		try {
			const value = await action();
			this.error = null;
			return value;
		} catch (e) {
			this.error = e instanceof Error ? e.message : String(e);
			return undefined;
		}
	}

	pause = (id: number) => this.run(() => unwrap(commands.pauseDownload(id)));
	resume = (id: number) => this.run(() => unwrap(commands.resumeDownload(id)));
	pauseAll = () => this.run(() => unwrap(commands.pauseAll()));
	resumeAll = () => this.run(() => unwrap(commands.resumeAll()));
	remove = (id: number, deleteFiles: boolean) =>
		this.run(() => unwrap(commands.removeDownload(id, deleteFiles)));
	removePackage = (id: number, deleteFiles: boolean) =>
		this.run(() => unwrap(commands.removePackage(id, deleteFiles)));
	open = (id: number) => this.run(() => unwrap(commands.openDownload(id)));
	reveal = (id: number) => this.run(() => unwrap(commands.revealDownload(id)));
	openFolder = (packageId: number) =>
		this.run(() => unwrap(commands.openPackageFolder(packageId)));
	/** A download's own speed limit in bytes per second; `null` removes it. */
	setSpeedLimit = (id: number, limit: number | null) =>
		this.run(() => unwrap(commands.setDownloadSpeedLimit(id, limit)));
	/** Put a package before another (`null`: last); the order is the start order. */
	movePackage = (id: number, before: number | null) =>
		this.run(() => unwrap(commands.movePackage(id, before)));

	#reindex() {
		// Index the reactive proxies so in-place updates re-render.
		this.#byId.clear();
		for (const p of this.packages) for (const d of p.downloads) this.#byId.set(d.id, d);
	}

	#apply(batch: DownloadEventBatch) {
		if (batch.resync) {
			void this.refresh();
			return;
		}
		let refresh = false;
		for (const event of batch.events) {
			switch (event.type) {
				case 'progress': {
					const d = this.#byId.get(event.downloadId);
					if (!d) break;
					d.downloaded = event.downloaded;
					if (event.total != null) d.size = event.total;
					if (event.files) d.files = event.files;
					this.live[event.downloadId] = {
						speedBps: event.speedBps,
						connections: event.connections
					};
					break;
				}
				case 'statusChanged': {
					const d = this.#byId.get(event.downloadId);
					if (!d) {
						refresh = true;
						break;
					}
					d.status = event.status;
					d.error = event.error;
					if (event.status === 'completed') {
						if (d.files) {
							// A gallery's size was an estimate; what it downloaded is the truth.
							d.size = d.downloaded;
							d.files.total = d.files.done;
						} else if (d.size != null) {
							d.downloaded = d.size;
						}
					}

					if (event.status !== 'active') delete this.live[event.downloadId];
					break;
				}
				case 'resolved': {
					const d = this.#byId.get(event.downloadId);
					if (!d) break;
					d.filename = event.filename;
					if (event.size != null) d.size = event.size;
					break;
				}
				case 'speedLimitChanged': {
					const d = this.#byId.get(event.downloadId);
					if (d) d.speedLimit = event.speedLimit;
					break;
				}
				case 'packageAdded':
				case 'packagesReordered':
					refresh = true;
					break;
				case 'downloadRemoved': {
					const p = this.package(event.packageId);
					if (p) p.downloads = p.downloads.filter((d) => d.id !== event.downloadId);
					this.#byId.delete(event.downloadId);
					delete this.live[event.downloadId];
					break;
				}
				case 'packageRemoved':
					this.packages = this.packages.filter((p) => p.id !== event.packageId);
					break;
			}
		}
		if (refresh) void this.refresh();
	}
}

export const downloads = new DownloadsStore();
