// Drive the running app's WebView2 over Chrome DevTools (see PLAN.md, "How to test in the real app").
// Start the app with WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=9222 --remote-allow-origins=*'.
//   node tools/dev/cdp.mjs eval "window.__TAURI_INTERNALS__.invoke('tools_list')"   (awaits promises, prints JSON)
//   node tools/dev/cdp.mjs shot out.png                                              (screenshot, solid background)
import { writeFileSync } from 'node:fs';

const [mode, arg] = process.argv.slice(2);
const pages = await (await fetch('http://127.0.0.1:9222/json')).json();
const page = pages.find((p) => p.type === 'page');
const ws = new WebSocket(page.webSocketDebuggerUrl);
let id = 0;
const pending = new Map();
ws.onmessage = (m) => {
	const msg = JSON.parse(m.data);
	if (pending.has(msg.id)) {
		pending.get(msg.id)(msg);
		pending.delete(msg.id);
	}
};
await new Promise((r) => (ws.onopen = r));
const send = (method, params = {}) =>
	new Promise((resolve) => {
		const n = ++id;
		pending.set(n, resolve);
		ws.send(JSON.stringify({ id: n, method, params }));
	});

if (mode === 'eval') {
	const r = await send('Runtime.evaluate', { expression: arg, awaitPromise: true, returnByValue: true });
	console.log(JSON.stringify(r.result?.result?.value ?? r.result?.exceptionDetails ?? r, null, 1));
} else if (mode === 'shot') {
	await send('Runtime.evaluate', { expression: 'delete document.documentElement.dataset.shell' });
	const r = await send('Page.captureScreenshot', { format: 'png' });
	writeFileSync(arg, Buffer.from(r.result.data, 'base64'));
	console.log('saved', arg);
}
ws.close();
