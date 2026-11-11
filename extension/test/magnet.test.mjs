// node --test extension/test
// Loads the content script src/magnet.js into a sandbox with stub browser
// objects, so the file itself is tested (it is a classic script, not a module).
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import vm from 'node:vm';

const source = readFileSync(new URL('../src/magnet.js', import.meta.url), 'utf8');
const MAGNET = 'magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567';

/** A fake element chain: `chain(['SPAN', ['A', href]])` is a span inside an anchor. */
function el(tagName, attrs = {}, parentElement = null) {
	return { tagName, parentElement, getAttribute: (name) => attrs[name] ?? null };
}

/**
 * Runs magnet.js in a fresh sandbox. `stored` is what storage.sync holds;
 * `reply` is what the background answers (undefined: no answer, as a
 * sendMessage failure would look).
 */
function load({ stored = {}, reply } = {}) {
	const listeners = {};
	const sent = [];
	const context = {
		location: { href: 'https://site.test/page' },
		document: {
			addEventListener(type, fn, capture) {
				listeners[type] = { fn, capture };
			}
		},
		chrome: {
			runtime: {
				lastError: undefined,
				sendMessage(message, callback) {
					sent.push(message);
					if (reply !== undefined) callback(reply);
				}
			},
			storage: {
				sync: { get: (_defaults, callback) => callback(stored) },
				onChanged: { addListener: () => {} }
			}
		}
	};
	vm.createContext(context);
	vm.runInContext(source, context);
	return { context, listeners, sent };
}

/** A click (or auxclick) on `target`; reports whether the default was prevented. */
function activate(listeners, type, target, button = 0) {
	const event = {
		type,
		button,
		target,
		prevented: false,
		stopped: false,
		preventDefault() {
			this.prevented = true;
		},
		stopPropagation() {
			this.stopped = true;
		}
	};
	listeners[type].fn(event);
	return event;
}

test('finds the magnet link an element is in, and nothing else', () => {
	const { context } = load();
	const anchor = el('A', { href: MAGNET });
	assert.equal(context.magnetLinkOf(anchor), MAGNET);
	assert.equal(context.magnetLinkOf(el('SPAN', {}, anchor)), MAGNET, 'inside the anchor');
	assert.equal(context.magnetLinkOf(el('STRONG', {}, el('a', { href: MAGNET }))), MAGNET, 'lower-case tag');
	assert.equal(context.magnetLinkOf(el('A', { href: '  Magnet:?xt=urn:btih:abc ' })), 'Magnet:?xt=urn:btih:abc', 'scheme in any case');
	assert.equal(context.magnetLinkOf(el('A', { href: 'https://a.test/x' })), '', 'web link');
	assert.equal(context.magnetLinkOf(el('SPAN', {}, el('A', { href: 'https://a.test/x' }))), '', 'web link around the element');
	assert.equal(context.magnetLinkOf(el('DIV', {}, el('BODY'))), '', 'no anchor at all');
	assert.equal(context.magnetLinkOf(el('A', {})), '', 'anchor without href');
});

test('listens for clicks and middle clicks in the capture phase', () => {
	const { listeners } = load();
	assert.equal(listeners.click.capture, true);
	assert.equal(listeners.auxclick.capture, true);
});

test('a click on a magnet link is stopped and sent to the app', () => {
	const { listeners, sent } = load({ reply: { ok: true } });
	const target = el('SPAN', {}, el('A', { href: MAGNET }));
	const event = activate(listeners, 'click', target);
	assert.equal(event.prevented, true);
	assert.equal(event.stopped, true);
	// JSON: the sandbox's objects come from another realm, which deepStrictEqual rejects.
	assert.equal(JSON.stringify(sent), JSON.stringify([{ type: 'magnet', url: MAGNET }]));
});

test('a middle click is caught too; other buttons and web links are left alone', () => {
	const { listeners, sent } = load({ reply: { ok: true } });
	const magnet = el('A', { href: MAGNET });
	assert.equal(activate(listeners, 'auxclick', magnet, 1).prevented, true);
	assert.equal(activate(listeners, 'auxclick', magnet, 2).prevented, false, 'right button');
	const web = activate(listeners, 'click', el('A', { href: 'https://a.test/x' }));
	assert.equal(web.prevented, false);
	assert.equal(sent.length, 1);
});

test('the setting off (read at start) leaves magnet clicks to the browser', () => {
	const { listeners, sent } = load({ stored: { magnetLinks: false }, reply: { ok: true } });
	assert.equal(activate(listeners, 'click', el('A', { href: MAGNET })).prevented, false);
	assert.equal(sent.length, 0);
});

test('the setting defaults to on when nothing is stored', () => {
	const { listeners, sent } = load({ stored: {}, reply: { ok: true } });
	activate(listeners, 'click', el('A', { href: MAGNET }));
	assert.equal(sent.length, 1);
});

test('when the app does not take the link, the browser opens it', () => {
	const refused = load({ reply: { ok: false } });
	activate(refused.listeners, 'click', el('A', { href: MAGNET }));
	assert.equal(refused.context.location.href, MAGNET);

	// No listener in the extension: the browser sets lastError and answers with nothing.
	const unreachable = load();
	unreachable.context.chrome.runtime.sendMessage = (message, callback) => {
		unreachable.context.chrome.runtime.lastError = { message: 'Could not establish connection.' };
		callback(undefined);
	};
	activate(unreachable.listeners, 'click', el('A', { href: MAGNET }));
	assert.equal(unreachable.context.location.href, MAGNET);
});

test('a send that throws (extension reloaded) falls back too', () => {
	const { context, listeners } = load();
	context.chrome.runtime.sendMessage = () => {
		throw new Error('Extension context invalidated.');
	};
	activate(listeners, 'click', el('A', { href: MAGNET }));
	assert.equal(context.location.href, MAGNET);
});

test('an accepted link leaves the page as it is', () => {
	const { context, listeners } = load({ reply: { ok: true } });
	activate(listeners, 'click', el('A', { href: MAGNET }));
	assert.equal(context.location.href, 'https://site.test/page');
});
