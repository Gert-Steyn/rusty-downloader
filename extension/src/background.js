// The extension's background: right-click menus, taking over downloads, and
// sending both to the app with the cookies their sites need.

import { AppError, call } from './native.js';
import {
	baseName,
	chunk,
	cookieProbes,
	describeError,
	linksInText,
	mergeCookies,
	normalizeSettings,
	shouldIntercept,
	stillMissingBytes,
	uniqueWebLinks
} from './rules.js';

const MENU = {
	link: 'send-link',
	page: 'send-page',
	allLinks: 'send-all-links',
	selection: 'send-selection'
};

chrome.runtime.onInstalled.addListener(() => {
	chrome.contextMenus.removeAll(() => {
		chrome.contextMenus.create({
			id: MENU.link,
			title: 'Download with Rusty Downloader',
			contexts: ['link']
		});
		chrome.contextMenus.create({
			id: MENU.selection,
			title: 'Send selected links to Rusty Downloader',
			contexts: ['selection']
		});
		chrome.contextMenus.create({
			id: MENU.allLinks,
			title: 'Send all links on this page to Rusty Downloader',
			contexts: ['page']
		});
		chrome.contextMenus.create({
			id: MENU.page,
			title: 'Send this page to Rusty Downloader (video or gallery)',
			contexts: ['page']
		});
	});
});

async function settings() {
	return normalizeSettings(await chrome.storage.sync.get(null));
}

chrome.contextMenus.onClicked.addListener((info, tab) => onMenu(info, tab));

// For tests over DevTools, which can't click a context menu: run a menu
// item as if clicked. Only code running inside the extension can reach it.
globalThis.rustyDownloader = { onMenu };

/** Send what a menu item is about; returns the totals (or the error), for tests. */
async function onMenu(info, tab) {
	try {
		const links = await linksFor(info, tab);
		if (links.length === 0) {
			notify('No links found', 'There were no http or https links to send.');
			return { added: 0, links };
		}
		const referer = tab?.url && /^https?:/.test(tab.url) ? tab.url : info.pageUrl;
		const reply = await send(links, { target: 'grabber', referer, tab });
		if (reply.added === 0 && reply.duplicates > 0) {
			notify('Already in the LinkGrabber', plural(reply.duplicates, 'link') + ' were there already.');
		}
		return { ...reply, links };
	} catch (e) {
		notify('Couldn’t send to Rusty Downloader', describeError(e));
		return { error: describeError(e), code: e.code };
	}
}

/** The links a menu item is about. */
async function linksFor(info, tab) {
	switch (info.menuItemId) {
		case MENU.link:
			return uniqueWebLinks([info.linkUrl]);
		case MENU.page:
			return uniqueWebLinks([info.pageUrl]);
		case MENU.allLinks:
			return uniqueWebLinks(await inPage(tab, () => [...document.links].map((a) => a.href), []));
		case MENU.selection: {
			const found = await inPage(tab, selectedLinks, null);
			// Pages scripts can't run in still have the selected text.
			return uniqueWebLinks(found ?? linksInText(info.selectionText || ''));
		}
		default:
			return [];
	}
}

/** Links inside the selection: anchors it touches, and links written as text. */
function selectedLinks() {
	const selection = getSelection();
	const links = [];
	for (let i = 0; i < selection.rangeCount; i++) {
		const range = selection.getRangeAt(i);
		const root = range.commonAncestorContainer;
		const scope = root.nodeType === Node.ELEMENT_NODE ? root : root.parentElement;
		const anchor = scope?.closest('a[href]');
		if (anchor) links.push(anchor.href);
		for (const a of scope?.querySelectorAll('a[href]') ?? []) {
			if (range.intersectsNode(a)) links.push(a.href);
		}
	}
	const text = selection.toString();
	for (const m of text.matchAll(/\bhttps?:\/\/[^\s"'<>]+/gi)) links.push(m[0].replace(/[),.;:!?\]}]+$/, ''));
	return links;
}

/** Run `func` in the tab's page; `fallback` where the browser doesn't allow that. */
async function inPage(tab, func, fallback) {
	if (!tab?.id) return fallback;
	try {
		const [result] = await chrome.scripting.executeScript({ target: { tabId: tab.id }, func });
		return result?.result ?? fallback;
	} catch {
		return fallback;
	}
}

/**
 * Send links to the app, in as many messages as needed, with the browser's
 * cookies for their sites (from the tab's cookie store) and its user agent.
 * Resolves with the totals.
 */
async function send(links, { target, referer, tab, filename, storeId }) {
	const prefs = await settings();
	const store = storeId ?? (await cookieStoreOf(tab));
	const totals = { added: 0, duplicates: 0, rejected: [] };
	for (const part of chunk(links)) {
		const cookies = prefs.sendCookies ? await cookiesFor(part, store) : [];
		const reply = await call({
			type: 'add',
			target,
			links: part,
			referer: referer || undefined,
			userAgent: navigator.userAgent,
			cookies,
			filename: links.length === 1 ? filename : undefined
		});
		totals.added += reply.added;
		totals.duplicates += reply.duplicates;
		totals.rejected.push(...reply.rejected);
	}
	return totals;
}

/** The cookie store a tab uses (a private window or Firefox container has its own). */
async function cookieStoreOf(tab) {
	if (!tab) return undefined;
	if (tab.cookieStoreId) return tab.cookieStoreId; // Firefox
	try {
		const stores = await chrome.cookies.getAllCookieStores();
		return stores.find((s) => s.tabIds.includes(tab.id))?.id;
	} catch {
		return undefined;
	}
}

/** The browser's cookies for the links' own sites, and only those. */
async function cookiesFor(links, storeId) {
	const lists = await Promise.all(
		cookieProbes(links).map((url) =>
			chrome.cookies.getAll(storeId ? { url, storeId } : { url }).catch(() => [])
		)
	);
	return mergeCookies(lists);
}

// Taking over downloads. A matching download is paused at once, so it can
// neither finish nor be lost while the app is asked; then it is cancelled
// if the app took it, or resumed if it couldn't.
//
// A small file can be fully received before the pause lands: Edge then
// reports it paused, finishes it anyway, and `cancel` succeeds without
// doing anything (seen in Edge 156). Erasing such a download would hide a
// second copy of the app's file. So only a download that is still missing
// bytes once paused is taken over; the rest stays with the browser.
chrome.downloads.onCreated.addListener(async (item) => {
	const prefs = await settings();
	const { take, why } = shouldIntercept(item, prefs);
	if (!take) return;
	try {
		await chrome.downloads.pause(item.id);
	} catch {
		return; // already finished, or not pausable: leave it to the browser
	}
	const [now] = await chrome.downloads.search({ id: item.id });
	if (!stillMissingBytes(now)) {
		if (now?.state === 'in_progress') await chrome.downloads.resume(item.id).catch(() => {});
		return;
	}
	const name = baseName(item.filename) || undefined;
	try {
		await send([item.url], {
			target: 'queue',
			referer: item.referrer,
			filename: name,
			storeId: item.cookieStoreId // Firefox containers
		});
		await chrome.downloads.cancel(item.id);
		const [after] = await chrome.downloads.search({ id: item.id });
		if (after?.state === 'complete') {
			// Finished after all: keep it visible rather than hide a copy.
			notify('Downloaded twice', `${name || item.url} is in the browser’s downloads and in Rusty Downloader.`);
			return;
		}
		await chrome.downloads.erase({ id: item.id }).catch(() => {});
		notify('Downloading with Rusty Downloader', `${name || item.url} (${why})`);
	} catch (e) {
		await chrome.downloads.resume(item.id).catch(() => {});
		if (e instanceof AppError && e.code === 'notInstalled') return; // nothing to tell yet
		notify('The browser kept this download', describeError(e));
	}
});

function notify(title, message) {
	chrome.notifications.create({
		type: 'basic',
		iconUrl: chrome.runtime.getURL('icons/128.png'),
		title,
		message
	});
}

function plural(n, word) {
	return `${n} ${word}${n === 1 ? '' : 's'}`;
}
