import { callOnce } from './native.js';
import { describeError, normalizeSettings } from './rules.js';

const $ = (id) => document.getElementById(id);

async function showStatus() {
	$('dot').className = 'dot';
	$('state').textContent = 'Checking…';
	$('detail').textContent = '';
	try {
		// `status` never starts the app, so opening the popup doesn't either.
		const s = await callOnce({ type: 'status' });
		$('dot').className = 'dot ok';
		$('state').textContent = 'Connected';
		$('detail').textContent =
			`${s.active} downloading, ${s.queued} waiting, ${s.grabber} in the LinkGrabber`;
	} catch (e) {
		$('dot').className = e.code === 'notRunning' ? 'dot' : 'dot bad';
		$('state').textContent =
			e.code === 'notRunning' ? 'Not running' : e.code === 'notInstalled' ? 'Not set up' : 'Not reachable';
		$('detail').textContent =
			e.code === 'notRunning' ? 'It starts by itself when you send something.' : describeError(e);
	}
}

async function showSettings() {
	const s = normalizeSettings(await chrome.storage.sync.get(null));
	$('intercept').checked = s.intercept;
	$('rule').textContent = s.intercept
		? `${s.extensions.length} file types` + (s.minSizeMB > 0 ? `, or ${s.minSizeMB} MB and up` : '')
		: 'The browser keeps every download.';
}

$('intercept').addEventListener('change', async (e) => {
	await chrome.storage.sync.set({ intercept: e.target.checked });
	showSettings();
});
$('options').addEventListener('click', () => chrome.runtime.openOptionsPage());
$('recheck').addEventListener('click', showStatus);

showStatus();
showSettings();
