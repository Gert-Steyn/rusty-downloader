// Content script (every frame): a click on a magnet link goes to Rusty
// Downloader instead of the browser's handler for magnet: links. A classic
// script with no imports, since it runs in every page; the test loads it into
// a sandbox (extension/test/magnet.test.mjs).

let enabled = true;

try {
	chrome.storage.sync.get({ magnetLinks: true }, (stored) => {
		enabled = stored?.magnetLinks !== false;
	});
	chrome.storage.onChanged.addListener((changes, area) => {
		if (area === 'sync' && 'magnetLinks' in changes) enabled = changes.magnetLinks.newValue !== false;
	});
} catch {
	// No storage (the extension was reloaded): keep the default.
}

/** The magnet link the element is in (its nearest anchor), or '' if there is none. */
function magnetLinkOf(node) {
	for (let el = node; el; el = el.parentElement) {
		if (String(el.tagName).toUpperCase() !== 'A') continue;
		const href = String(el.getAttribute('href') || '').trim();
		return /^magnet:/i.test(href) ? href : '';
	}
	return '';
}

function onActivate(event) {
	if (!enabled) return;
	// Middle click only: the other buttons are left to the page and the browser.
	if (event.type === 'auxclick' && event.button !== 1) return;
	const url = magnetLinkOf(event.target);
	if (!url) return;
	event.preventDefault();
	event.stopPropagation();
	send(url);
}

/** Hand the link to the app; if the app can't take it, let the browser open it as usual. */
function send(url) {
	const fallBack = () => {
		location.href = url;
	};
	try {
		chrome.runtime.sendMessage({ type: 'magnet', url }, (reply) => {
			if (chrome.runtime.lastError || !reply?.ok) fallBack();
		});
	} catch {
		fallBack();
	}
}

document.addEventListener('click', onActivate, true);
document.addEventListener('auxclick', onActivate, true);
