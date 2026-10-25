// Build the extension for each browser family into extension/dist/:
//   chromium/  Chrome, Edge, Brave (load unpacked)
//   firefox/   Firefox (about:debugging → Load Temporary Add-on → manifest.json)
// The sources in src/ are shared; only the manifest differs. Icons come from the app.
//
//   node extension/build.mjs          build both
//   node extension/build.mjs --check  only check that the pinned extension ids match the
//                                     ones the app registers the native host for (CI runs this)
import { createHash } from 'node:crypto';
import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const src = join(here, 'src');
const dist = join(here, 'dist');
const appIcons = join(here, '..', 'src-tauri', 'icons');
const registration = join(here, '..', 'src-tauri', 'src', 'native_host.rs');

export const FIREFOX_ID = 'extension@rustydownloader.dev';

const manifest = JSON.parse(readFileSync(join(src, 'manifest.json'), 'utf8'));

/** Chromium derives an unpacked extension's id from the manifest's public key. */
export function chromiumId(key) {
	const hash = createHash('sha256').update(Buffer.from(key, 'base64')).digest('hex');
	return [...hash.slice(0, 32)].map((c) => String.fromCharCode(97 + parseInt(c, 16))).join('');
}

function firefoxManifest(m) {
	const { key, minimum_chrome_version, background, ...rest } = m;
	return {
		...rest,
		background: { scripts: [background.service_worker], type: 'module' },
		browser_specific_settings: { gecko: { id: FIREFOX_ID, strict_min_version: '128.0' } }
	};
}

function check() {
	const rust = readFileSync(registration, 'utf8');
	const id = chromiumId(manifest.key);
	const problems = [];
	if (!rust.includes(`"${id}"`)) problems.push(`native_host.rs doesn't allow the Chromium id ${id}`);
	if (!rust.includes(`"${FIREFOX_ID}"`)) problems.push(`native_host.rs doesn't allow the Firefox id ${FIREFOX_ID}`);
	if (problems.length) {
		console.error(problems.join('\n'));
		process.exit(1);
	}
	console.log(`extension ids match the app: ${id}, ${FIREFOX_ID}`);
}

function build() {
	rmSync(dist, { recursive: true, force: true });
	for (const [browser, m] of [
		['chromium', manifest],
		['firefox', firefoxManifest(manifest)]
	]) {
		const out = join(dist, browser);
		cpSync(src, out, { recursive: true });
		writeFileSync(join(out, 'manifest.json'), JSON.stringify(m, null, '\t') + '\n');
		mkdirSync(join(out, 'icons'), { recursive: true });
		for (const size of [32, 64, 128]) {
			cpSync(join(appIcons, `${size}x${size}.png`), join(out, 'icons', `${size}.png`));
		}
		console.log(`built ${out}`);
	}
	console.log(`Chromium id ${chromiumId(manifest.key)}, Firefox id ${FIREFOX_ID}`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
	if (process.argv.includes('--check')) check();
	else {
		check();
		build();
	}
}
