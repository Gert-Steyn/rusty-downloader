// Pure helpers: settings, which downloads to take over, and finding links.
// No browser APIs here, so `node --test extension/test` can check them.

/** Most links in one message (the app's limit). */
export const MAX_LINKS = 1000;

/** Sites whose cookies are read for one send; more are sent without cookies. */
export const MAX_COOKIE_SITES = 25;

export const DEFAULT_SETTINGS = Object.freeze({
	/** Take over browser downloads that match the rules below. */
	intercept: true,
	/** Downloads at least this big are taken over (0: size alone never decides). */
	minSizeMB: 20,
	/** File types that are always taken over (any size). */
	extensions: [
		'zip', '7z', 'rar', 'iso', 'exe', 'msi', 'tar', 'gz', 'xz', 'bz2', 'zst',
		'mp4', 'mkv', 'avi', 'mov', 'webm', 'mp3', 'flac', 'm4a', 'apk', 'dmg', 'img'
	],
	/** Sites whose downloads are always taken over. */
	alwaysSites: [],
	/** Sites whose downloads the browser keeps. */
	neverSites: [],
	/** Send the browser's cookies for the links' sites. */
	sendCookies: true,
	/** Clicks on magnet links go to the app instead of the browser's torrent handler. */
	magnetLinks: true
});

/** Settings as stored, with defaults for anything missing or malformed. */
export function normalizeSettings(stored = {}) {
	const s = { ...DEFAULT_SETTINGS };
	if (typeof stored.intercept === 'boolean') s.intercept = stored.intercept;
	if (typeof stored.sendCookies === 'boolean') s.sendCookies = stored.sendCookies;
	if (typeof stored.magnetLinks === 'boolean') s.magnetLinks = stored.magnetLinks;
	const mb = Number(stored.minSizeMB);
	if (Number.isFinite(mb) && mb >= 0) s.minSizeMB = mb;
	for (const key of ['extensions', 'alwaysSites', 'neverSites']) {
		if (Array.isArray(stored[key])) s[key] = cleanList(stored[key], key === 'extensions');
	}
	return s;
}

/** Trimmed, lowercased, de-duplicated entries; file types lose a leading dot. */
export function cleanList(items, isExtension = false) {
	const out = [];
	for (let item of items) {
		item = String(item).trim().toLowerCase();
		if (isExtension) item = item.replace(/^\*?\./, '');
		else item = item.replace(/^\*\./, '').replace(/^\.+/, '');
		if (item && !out.includes(item)) out.push(item);
	}
	return out;
}

/** Split a text box (one entry per line, or separated by commas or spaces). */
export function parseList(text, isExtension = false) {
	return cleanList(String(text).split(/[\s,;]+/), isExtension);
}

/** `host` is `site` or one of its subdomains. */
export function hostMatches(host, site) {
	host = host.toLowerCase();
	return host === site || host.endsWith('.' + site);
}

function hostOf(url) {
	try {
		return new URL(url).hostname;
	} catch {
		return '';
	}
}

export function isWebUrl(url) {
	try {
		const { protocol } = new URL(url);
		return protocol === 'http:' || protocol === 'https:';
	} catch {
		return false;
	}
}

export function isMagnetUrl(url) {
	return /^magnet:/i.test(String(url ?? '').trim());
}

/** Links the app takes in an `add`: web links and magnet links. Others are never sent. */
export function isSendableUrl(url) {
	return isWebUrl(url) || isMagnetUrl(url);
}

/** The file type of a download: from its file name, else its URL's path. */
export function extensionOf(filename, url) {
	const name = baseName(filename) || (() => {
		try {
			return decodeURIComponent(new URL(url).pathname.split('/').pop() || '');
		} catch {
			return '';
		}
	})();
	const dot = name.lastIndexOf('.');
	return dot > 0 ? name.slice(dot + 1).toLowerCase() : '';
}

/** Last part of a Windows or Unix path. */
export function baseName(path) {
	return String(path || '').split(/[\\/]/).pop();
}

/**
 * Whether a browser download should go to the app instead, and why (for logs).
 * `item` is shaped like a `downloads.DownloadItem`.
 */
export function shouldIntercept(item, settings) {
	if (!settings.intercept) return { take: false, why: 'switched off' };
	if (item.incognito) return { take: false, why: 'private window' };
	if (!isWebUrl(item.url)) return { take: false, why: 'not a web link' };
	const host = hostOf(item.finalUrl || item.url);
	if (settings.neverSites.some((s) => hostMatches(host, s))) return { take: false, why: 'site kept in browser' };
	if (settings.alwaysSites.some((s) => hostMatches(host, s))) return { take: true, why: 'site always sent' };
	// Torrent files are always taken over, whatever their size.
	const ext = extensionOf(item.filename, item.finalUrl || item.url);
	if (ext === 'torrent' || String(item.mime || '').toLowerCase() === 'application/x-bittorrent') {
		return { take: true, why: '.torrent file' };
	}
	if (ext && settings.extensions.includes(ext)) return { take: true, why: `.${ext} file` };
	const size = item.totalBytes > 0 ? item.totalBytes : item.fileSize > 0 ? item.fileSize : 0;
	if (settings.minSizeMB > 0 && size >= settings.minSizeMB * 1024 * 1024) {
		return { take: true, why: `${Math.round(size / 1048576)} MB` };
	}
	return { take: false, why: 'no rule matched' };
}

/**
 * A paused download the browser can't finish by itself: part of the file is
 * still to come. (One that was fully received before the pause landed is
 * finished by the browser regardless.)
 */
export function stillMissingBytes(item) {
	if (!item || item.state !== 'in_progress' || !item.paused) return false;
	const total = item.totalBytes > 0 ? item.totalBytes : item.fileSize;
	// Unknown size: nothing says the browser has it all.
	return !(total > 0) || item.bytesReceived < total;
}

const LINK = /\b(?:https?:\/\/|magnet:\?)[^\s"'<>]+/gi;

/** http(s) and magnet links in plain text, without trailing punctuation from prose. */
export function linksInText(text) {
	return [...String(text).matchAll(LINK)].map((m) => m[0].replace(/[),.;:!?\]}]+$/, ''));
}

/** Sendable links only, de-duplicated, in order, without `#fragment`-only differences. */
export function uniqueWebLinks(urls) {
	const seen = new Set();
	const out = [];
	for (const raw of urls) {
		if (!isSendableUrl(raw)) continue;
		const url = new URL(raw);
		url.hash = '';
		const key = url.href;
		if (!seen.has(key)) {
			seen.add(key);
			out.push(raw);
		}
	}
	return out;
}

/** Split into messages the app accepts. */
export function chunk(items, size = MAX_LINKS) {
	const out = [];
	for (let i = 0; i < items.length; i += size) out.push(items.slice(i, i + size));
	return out;
}

/**
 * One URL per site whose cookies to read, at most `limit` sites. Cookies are
 * read for that URL, which covers the site's domain-wide cookies and those
 * for the path the links are under.
 */
export function cookieProbes(urls, limit = MAX_COOKIE_SITES) {
	const bySite = new Map();
	for (const url of urls) {
		const host = hostOf(url);
		if (host && !bySite.has(host)) bySite.set(host, url);
	}
	return [...bySite.values()].slice(0, limit);
}

/** Merge cookie lists, keeping one per (name, domain, path). */
export function mergeCookies(lists) {
	const seen = new Map();
	for (const list of lists) {
		for (const c of list) {
			const key = `${c.name}\t${c.domain}\t${c.path}`;
			if (!seen.has(key)) seen.set(key, c);
		}
	}
	return [...seen.values()];
}

/** What a reply means for the user. */
export function describeError(error) {
	switch (error?.code) {
		case 'notInstalled':
			return 'Rusty Downloader isn’t installed for this browser. Start the app once to set it up.';
		case 'notRunning':
			return error.message || 'Rusty Downloader isn’t running.';
		case 'untrusted':
			return 'Something other than Rusty Downloader is answering. Nothing was sent.';
		default:
			return error?.message || String(error);
	}
}
