// gallery-dl helpers: the options drawer's flag schema (loaded once) and
// short descriptions of galleries and their settings.
import { commands, isTauri, unwrap, type Gallery, type GalleryDlFlags, type GalleryDlOptions } from './api.js';

export const DEFAULT_GALLERY_OPTIONS: GalleryDlOptions = {
	archive: true,
	userConfig: false,
	flags: {},
	rawArgs: null,
	keepFolders: false,
	folderPattern: null
};

let flags: Promise<GalleryDlFlags> | null = null;

/** The flag schema, plus which flags the installed gallery-dl knows. */
export function galleryFlags(): Promise<GalleryDlFlags> {
	if (!flags) {
		flags = isTauri()
			? unwrap(commands.gallerydlFlags())
			: import('./mock.js').then((m) => m.galleryFlags());
		// Let a failed load be retried next time.
		flags.catch(() => (flags = null));
	}
	return flags;
}

function count(n: number, one: string, many: string) {
	return `${n.toLocaleString()} ${n === 1 ? one : many}`;
}

/** "24 files · imgur album", "12+ albums", "2,000+ files". */
export function galleryLine(g: Gallery): string {
	const counts: string[] = [];
	if (g.files || !g.subGalleries) {
		counts.push(g.partial ? `${g.files ? g.files.toLocaleString() : 'Many'}+ files` : count(g.files, 'file', 'files'));
	}
	if (g.subGalleries) counts.push(count(g.subGalleries, 'gallery', 'galleries'));
	const source = [g.category, g.subcategory].filter(Boolean).join(' ');
	return [...counts, source].filter(Boolean).join(' · ');
}

/** How many settings differ from the defaults, for the options button. */
export function changedCount(o: GalleryDlOptions | null | undefined): number {
	if (!o) return 0;
	let n = Object.values(o.flags).filter((v) => v.trim() && v !== 'false').length;
	if (!o.archive) n++;
	if (o.userConfig) n++;
	if (o.rawArgs?.trim()) n++;
	if (o.keepFolders) n++;
	return n;
}

export function sameOptions(a: GalleryDlOptions, b: GalleryDlOptions): boolean {
	const clean = (o: GalleryDlOptions) =>
		JSON.stringify({
			...o,
			rawArgs: o.rawArgs?.trim() || null,
			folderPattern: o.folderPattern?.trim() || null,
			flags: Object.fromEntries(
				Object.entries(o.flags)
					.filter(([, v]) => v.trim() && v !== 'false')
					.sort(([x], [y]) => x.localeCompare(y))
			)
		});
	return clean(a) === clean(b);
}
