// Drive a test Edge (or Chrome) started with the extension over DevTools (see PLAN.md, "How 2d works").
// Start it with a throwaway profile, the unpacked extension and a debugging port:
//   msedge.exe --user-data-dir=<temp> --load-extension=<repo>\extension\dist\chromium
//     --disable-features=DisableLoadExtensionCommandLineSwitch --remote-debugging-port=9333 --no-first-run <url>
// Then, with <target> any part of a target's URL (page, extension page or the extension's worker):
//   node tools/dev/edge.mjs <target> "<js>"                          evaluate (awaits promises, prints JSON)
//   node tools/dev/edge.mjs <target> 'cdp:Page.navigate:{"url":"…"}'  raw DevTools calls, " ;; " between several
//   node tools/dev/edge.mjs <target> shot:out.png                     screenshot (the tab must be in front)
// From Git Bash, leave the leading "/" off targets ("popup.html"): a "/x" argument becomes a Windows path.
const [match, expression] = process.argv.slice(2);
const targets = await (await fetch(`http://127.0.0.1:${process.env.CDP_PORT ?? 9333}/json/list`)).json();
const target = targets.find((t) => t.url.includes(match));
if (!target) {
	console.error('no target matching', match, targets.map((t) => t.url));
	process.exit(1);
}
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
const reply = new Promise(
	(r) =>
		(ws.onmessage = (m) => {
			const msg = JSON.parse(m.data);
			if (msg.id === 1) r(msg);
		})
);
if (expression.startsWith('cdp:')) {
	// cdp:<Method>:<json params>, several separated by " ;; "
	let n = 1;
	for (const call of expression.slice(4).split(' ;; ')) {
		const [method, ...rest] = call.split(':');
		const id = ++n;
		const done = new Promise((r) => {
			ws.onmessage = (m) => {
				const msg = JSON.parse(m.data);
				if (msg.id === id) r(msg);
			};
		});
		ws.send(JSON.stringify({ id, method, params: rest.length ? JSON.parse(rest.join(':')) : {} }));
		console.log(method, JSON.stringify((await done).result ?? {}));
	}
} else if (expression.startsWith('shot:')) {
	ws.send(JSON.stringify({ id: 1, method: 'Page.captureScreenshot', params: { format: 'png' } }));
	const msg = await reply;
	(await import('node:fs')).writeFileSync(expression.slice(5), Buffer.from(msg.result.data, 'base64'));
	console.log('saved', expression.slice(5));
} else {
	ws.send(JSON.stringify({ id: 1, method: 'Runtime.evaluate', params: { expression, awaitPromise: true, returnByValue: true } }));
	const msg = await reply;
	console.log(JSON.stringify(msg.result?.result?.value ?? msg.result?.exceptionDetails ?? msg, null, 1));
}
ws.close();
