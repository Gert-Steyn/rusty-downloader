// Talking to the app through the native host (`rdl-native-host.exe`).
//
// One port per browser session: the host keeps its pipe connection open
// between messages. The port is closed after a few idle minutes, which lets
// the host exit and the service worker sleep; the next message opens it
// again. Replies are matched to requests by `id`.

export const HOST = 'dev.rustydownloader.host';
const PROTOCOL = 1;
const IDLE_MS = 3 * 60 * 1000;
/** Starting the app can take up to 20 s, then the app has 30 s to answer. */
const TIMEOUT_MS = 60 * 1000;

/** An `error` reply, or a failure to reach the host. */
export class AppError extends Error {
	constructor(message, code) {
		super(message);
		this.code = code;
	}
}

/** The browser couldn't start the host at all: not registered (or not allowed). */
function hostMissing(message) {
	return /not found|forbidden|not allowed|no such native/i.test(message);
}

let port = null;
let nextId = 1;
const pending = new Map();
let idleTimer = null;

function open() {
	if (port) return port;
	port = chrome.runtime.connectNative(HOST);
	port.onMessage.addListener((reply) => {
		const waiter = pending.get(reply?.id);
		if (!waiter) return;
		pending.delete(reply.id);
		waiter.settle(reply);
	});
	port.onDisconnect.addListener(() => {
		const message = chrome.runtime.lastError?.message || 'The connection to Rusty Downloader closed.';
		const error = hostMissing(message)
			? new AppError(message, 'notInstalled')
			: new AppError(message, 'unavailable');
		port = null;
		for (const waiter of pending.values()) waiter.fail(error);
		pending.clear();
	});
	return port;
}

function touch() {
	clearTimeout(idleTimer);
	idleTimer = setTimeout(() => {
		if (pending.size === 0 && port) {
			port.disconnect();
			port = null;
		}
	}, IDLE_MS);
}

/** Send one request; resolves with the reply, rejects with an `AppError`. */
export function call(body) {
	const id = nextId++;
	return new Promise((resolve, reject) => {
		const timer = setTimeout(() => {
			pending.delete(id);
			reject(new AppError('Rusty Downloader took too long to answer.', 'unavailable'));
		}, TIMEOUT_MS);
		pending.set(id, {
			settle(reply) {
				clearTimeout(timer);
				if (reply.type === 'error') reject(new AppError(reply.message, reply.code));
				else resolve(reply);
			},
			fail(error) {
				clearTimeout(timer);
				reject(error);
			}
		});
		try {
			open().postMessage({ v: PROTOCOL, id, ...body });
		} catch (e) {
			pending.delete(id);
			clearTimeout(timer);
			reject(new AppError(e.message, 'unavailable'));
		}
		touch();
	});
}

/**
 * One request on its own host process, for pages other than the background
 * (the popup): never starts the app, since only `add` does.
 */
export async function callOnce(body) {
	try {
		const reply = await chrome.runtime.sendNativeMessage(HOST, { v: PROTOCOL, id: 1, ...body });
		if (reply?.type === 'error') throw new AppError(reply.message, reply.code);
		return reply;
	} catch (e) {
		if (e instanceof AppError) throw e;
		const message = e?.message || String(e);
		throw new AppError(message, hostMissing(message) ? 'notInstalled' : 'unavailable');
	}
}
