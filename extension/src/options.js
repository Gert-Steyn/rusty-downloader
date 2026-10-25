import { normalizeSettings, parseList } from './rules.js';

const $ = (id) => document.getElementById(id);
const ALL_SITES = { origins: ['<all_urls>'] };
const LISTS = ['extensions', 'alwaysSites', 'neverSites'];

async function load() {
	const s = normalizeSettings(await chrome.storage.sync.get(null));
	$('intercept').checked = s.intercept;
	$('sendCookies').checked = s.sendCookies;
	$('minSizeMB').value = s.minSizeMB;
	for (const key of LISTS) $(key).value = s[key].join('\n');
	// Firefox asks for site access separately from installing.
	$('access').hidden = await chrome.permissions.contains(ALL_SITES);
}

async function save() {
	const stored = {
		intercept: $('intercept').checked,
		sendCookies: $('sendCookies').checked,
		minSizeMB: Number($('minSizeMB').value) || 0
	};
	for (const key of LISTS) stored[key] = parseList($(key).value, key === 'extensions');
	await chrome.storage.sync.set(normalizeSettings(stored));
	$('saved').textContent = 'Saved.';
	setTimeout(() => ($('saved').textContent = ''), 1500);
}

for (const id of ['intercept', 'sendCookies', 'minSizeMB', ...LISTS]) {
	$(id).addEventListener('change', save);
}
$('grant').addEventListener('click', async () => {
	await chrome.permissions.request(ALL_SITES);
	load();
});

load();
