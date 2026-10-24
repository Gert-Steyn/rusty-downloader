// Live view of the LinkGrabber: staged links, grouped into packages, kept in
// sync by `grabberEvents`. Links are ticked for download by default; offline
// ones start unticked.
import { SvelteSet } from 'svelte/reactivity';
import {
	commands,
	events,
	isTauri,
	unwrap,
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
	playlist: ' from the playlist'
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
	 * here, which gets a "Review" button on other pages.
	 */
	notice = $state<{ text: string; id: number; review: boolean } | null>(null);

	groups = $derived.by(() => {
		const map = new Map<string, LinkGroup>();
		for (const link of this.links) {
			const key = `${link.package}\u0000${link.destDir ?? ''}`;
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

	/** Queue the ticked links. Returns the number of packages created. */
	async confirm(): Promise<number> {
		const ids = this.checkedIds;
		if (!ids.length) return 0;
		const packages = await this.run(() => unwrap(commands.grabberConfirm(ids)));
		if (!packages) return 0;
		this.say(`${plural(ids.length, 'download')} queued in ${plural(packages.length, 'package')}`);
		return packages.length;
	}

	async setClipboardWatch(enabled: boolean) {
		this.clipboardWatch = enabled;
		if (isTauri()) await this.run(() => unwrap(commands.setClipboardWatch(enabled)));
	}

	#setLinks(links: GrabbedLink[]) {
		this.links = links;
		for (const l of links) if (l.availability === 'offline') this.unchecked.add(l.id);
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
					// Manual adds already announced themselves in `add`.
					if (event.source !== 'manual') {
						this.say(`${plural(event.links.length, 'link')} added${SOURCE_LABEL[event.source]}`, true);
					}
					break;
				}
				case 'updated': {
					const i = this.links.findIndex((l) => l.id === event.link.id);
					if (i >= 0) this.links[i] = event.link;
					if (event.link.availability === 'offline') this.unchecked.add(event.link.id);
					break;
				}
				case 'removed': {
					const gone = new Set(event.ids);
					this.links = this.links.filter((l) => !gone.has(l.id));
					for (const id of event.ids) this.unchecked.delete(id);
					break;
				}
			}
		}
	}
}

export const grabber = new GrabberStore();
