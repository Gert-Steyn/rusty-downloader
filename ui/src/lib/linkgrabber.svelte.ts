// Live view of the LinkGrabber: staged links, grouped into packages, kept in
// sync by `grabberEvents`. Links are ticked for download by default; offline
// ones start unticked.
import { SvelteSet } from 'svelte/reactivity';
import {
	commands,
	events,
	isTauri,
	unwrap,
	type GalleryDlOptions,
	type GrabResult,
	type GrabbedLink,
	type GrabberEventBatch,
	type LinkSource,
	type YtDlpOptions
} from './api.js';

export interface LinkGroup {
	key: string;
	name: string;
	destDir: string | null;
	links: GrabbedLink[];
}

const SOURCE_LABEL: Record<LinkSource, string> = {
	manual: '',
	clipboard: ' from the clipboard',
	drop: ' from the drop',
	playlist: ' from the playlist',
	browser: ' from the browser'
};

function plural(n: number, word: string) {
	return `${n} ${word}${n === 1 ? '' : 's'}`;
}

class GrabberStore {
	links = $state<GrabbedLink[]>([]);
	/** Ids the user unticked (everything else will be downloaded). */
	unchecked = new SvelteSet<number>();
	clipboardWatch = $state(true);
	ready = $state(false);
	error = $state<string | null>(null);
	/**
	 * Short message for the toast. `review` marks news about links waiting
	 * here, which gets a "Review" button on other pages. `addText` is a
	 * clipboard offer: its button adds those links.
	 */
	notice = $state<{ text: string; id: number; review: boolean; addText?: string } | null>(null);

	groups = $derived.by(() => {
		const map = new Map<string, LinkGroup>();
		for (const link of this.links) {
			// One category per name (ignoring case): its links share one folder.
			const key = link.package.toLowerCase();
			let group = map.get(key);
			if (!group) {
				group = { key, name: link.package, destDir: link.destDir, links: [] };
				map.set(key, group);
			}
			group.links.push(link);
		}
		return [...map.values()];
	});

	checkedIds = $derived(this.links.filter((l) => !this.unchecked.has(l.id)).map((l) => l.id));

	#started = false;
	#noticeId = 0;

	async start() {
		if (this.#started) return;
		this.#started = true;
		if (!isTauri()) {
			if (import.meta.env.DEV) {
				const mock = await import('./mock.js');
				this.#setLinks(mock.grabbedLinks());
			}
			this.ready = true;
			return;
		}
		await events.grabberEvents.listen((e) => this.#apply(e.payload));
		await events.clipboardOffer.listen((e) => this.offer(e.payload.text, e.payload.count));

		this.clipboardWatch = await commands.getClipboardWatch();
		await this.refresh();
	}

	async refresh() {
		try {
			this.#setLinks(await unwrap(commands.grabberList()));
			this.error = null;
		} catch (e) {
			this.error = String(e);
		} finally {
			this.ready = true;
		}
	}

	isChecked(id: number) {
		return !this.unchecked.has(id);
	}

	setChecked(ids: number[], checked: boolean) {
		for (const id of ids) {
			if (checked) this.unchecked.delete(id);
			else this.unchecked.add(id);
		}
	}

	say(text: string, review = false) {
		this.notice = { text, id: ++this.#noticeId, review };
	}

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

	/** Stage the links found in `text`. Returns how many were added. */
	async add(text: string, source: LinkSource): Promise<number> {
		const result = await this.run(() => unwrap(commands.grabberAdd(text, source)));
		return this.#announce(result, source);
	}

	/** Stage a `.torrent` file dropped on the window. Returns how many were added. */
	async addTorrentFile(file: File): Promise<number> {
		const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
		const result = await this.run(() => unwrap(commands.grabberAddTorrentFile(file.name, bytes)));
		return this.#announce(result, 'drop');
	}

	/** Stage a `.torrent` file picked in the open dialog. Returns how many were added. */
	async addTorrentPath(path: string): Promise<number> {
		const result = await this.run(() => unwrap(commands.grabberAddTorrentPath(path)));
		return this.#announce(result, 'manual');
	}

	#announce(result: GrabResult | undefined, source: LinkSource): number {
		if (!result) return 0;
		const dupes = result.duplicates ? `, ${result.duplicates} already here` : '';
		this.say(`${plural(result.added, 'link')} added${SOURCE_LABEL[source]}${dupes}`, result.added > 0);
		return result.added;
	}

	remove = (ids: number[]) => this.run(() => unwrap(commands.grabberRemove(ids)));
	clear = () => this.run(() => unwrap(commands.grabberClear()));
	rename = (id: number, name: string) => this.run(() => unwrap(commands.grabberRename(id, name)));
	setPackage = (ids: number[], name: string) =>
		this.run(() => unwrap(commands.grabberSetPackage(ids, name)));
	setFolder = (ids: number[], dir: string | null) =>
		this.run(() => unwrap(commands.grabberSetFolder(ids, dir)));
	recheck = (ids: number[]) => this.run(() => unwrap(commands.grabberRecheck(ids)));
	setFormat = (ids: number[], options: YtDlpOptions) =>
		this.run(() => unwrap(commands.grabberSetFormat(ids, options)));
	setGalleryOptions = (ids: number[], options: GalleryDlOptions) =>
		this.run(() => unwrap(commands.grabberSetGalleryOptions(ids, options)));
	/** The chosen file indices of torrent links, or `null` for all of them. */
	setTorrentFiles = (ids: number[], onlyFiles: number[] | null) =>
		this.run(() => unwrap(commands.grabberSetTorrentFiles(ids, onlyFiles)));

	/**
	 * Queue the ticked links. Links still being checked stay in the LinkGrabber
	 * (the core holds them back). Returns the number of packages created.
	 */
	async confirm(): Promise<number> {
		const ids = this.checkedIds;
		if (!ids.length) return 0;
		const heldIds = ids.filter((id) => {
			const link = this.links.find((l) => l.id === id);
			return link?.availability === 'unknown' || link?.availability === 'checking';
		});
		if (heldIds.length === ids.length) {
			this.say('Those links are still being checked. Start them when the check finishes');
			return 0;
		}
		const packages = await this.run(() => unwrap(commands.grabberConfirm(ids)));
		if (!packages) return 0;
		const queued = ids.length - heldIds.length;
		let text = `${plural(queued, 'download')} queued in ${plural(packages.length, 'package')}`;
		if (heldIds.length) {
			const stays = heldIds.length === 1 ? 'is' : 'are';
			text += `. ${plural(heldIds.length, 'link')} ${stays} still being checked and ${heldIds.length === 1 ? 'stays' : 'stay'} here`;
		}
		this.say(text);
		return packages.length;
	}

	async setClipboardWatch(enabled: boolean) {
		this.clipboardWatch = enabled;
		if (isTauri()) await this.run(() => unwrap(commands.setClipboardWatch(enabled)));
	}

	#setLinks(links: GrabbedLink[]) {
		this.links = links;
		for (const l of links) this.#autoUntick(l);
	}

	/** Links whose earlier download is still on disk were unticked once already. */
	#flagged = new Set<number>();

	/**
	 * Offline links start unticked, and so do links downloaded before whose
	 * file is still there, but only the first time, so ticking one back sticks.
	 */
	#autoUntick(link: GrabbedLink) {
		if (link.availability === 'offline') this.unchecked.add(link.id);
		if (link.previous?.exists && !this.#flagged.has(link.id)) {
			this.#flagged.add(link.id);
			this.unchecked.add(link.id);
		}
	}

	/** Copied links waiting for a yes ("ask first" clipboard mode). */
	offer(text: string, count: number) {
		this.notice = {
			text: `${plural(count, 'link')} copied`,
			id: ++this.#noticeId,
			review: false,
			addText: text
		};
	}

	#apply(batch: GrabberEventBatch) {
		if (batch.resync) {
			void this.refresh();
			return;
		}
		for (const event of batch.events) {
			switch (event.type) {
				case 'added': {
					const known = new Set(this.links.map((l) => l.id));
					this.links.push(...event.links.filter((l) => !known.has(l.id)));
					for (const l of event.links) this.#autoUntick(l);
					// Manual adds already announced themselves in `add`.
					if (event.source !== 'manual') {
						this.say(`${plural(event.links.length, 'link')} added${SOURCE_LABEL[event.source]}`, true);
					}
					break;
				}
				case 'updated': {
					const i = this.links.findIndex((l) => l.id === event.link.id);
					if (i >= 0) this.links[i] = event.link;
					this.#autoUntick(event.link);
					break;
				}
				case 'removed': {
					const gone = new Set(event.ids);
					this.links = this.links.filter((l) => !gone.has(l.id));
					for (const id of event.ids) {
						this.unchecked.delete(id);
						this.#flagged.delete(id);
					}
					break;
				}
			}
		}
	}
}

export const grabber = new GrabberStore();
