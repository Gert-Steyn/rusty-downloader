// node --test extension/test
import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
	DEFAULT_SETTINGS,
	chunk,
	cookieProbes,
	extensionOf,
	hostMatches,
	isMagnetUrl,
	isSendableUrl,
	linksInText,
	mergeCookies,
	normalizeSettings,
	parseList,
	shouldIntercept,
	stillMissingBytes,
	uniqueWebLinks
} from '../src/rules.js';
import { chromiumId } from '../build.mjs';

const settings = (over = {}) => normalizeSettings({ ...DEFAULT_SETTINGS, ...over });
const item = (over = {}) => ({
	id: 1,
	url: 'https://files.example.com/pub/tool.zip',
	finalUrl: '',
	filename: '',
	totalBytes: 1000,
	fileSize: 0,
	incognito: false,
	...over
});

test('takes over downloads by type, size and site', () => {
	const s = settings({ minSizeMB: 10, neverSites: ['bank.example'], alwaysSites: ['mirror.test'] });
	assert.equal(shouldIntercept(item(), s).take, true, 'zip by its URL');
	assert.equal(
		shouldIntercept(item({ url: 'https://x.test/get?id=1', filename: 'C:\\Users\\me\\Downloads\\a.ISO' }), s).take,
		true,
		'type from the file name'
	);
	assert.equal(shouldIntercept(item({ url: 'https://x.test/notes.txt' }), s).take, false);
	assert.equal(shouldIntercept(item({ url: 'https://x.test/notes.txt', totalBytes: 11 * 1048576 }), s).take, true);
	assert.equal(shouldIntercept(item({ url: 'https://x.test/a.txt', totalBytes: 0, fileSize: 20 * 1048576 }), s).take, true);
	assert.equal(shouldIntercept(item({ url: 'https://dl.bank.example/statement.zip' }), s).take, false, 'never-site wins');
	assert.equal(shouldIntercept(item({ url: 'https://a.mirror.test/readme.txt' }), s).take, true, 'always-site');
	assert.equal(
		shouldIntercept(item({ url: 'https://x.test/r', finalUrl: 'https://dl.bank.example/s.zip' }), s).take,
		false,
		'site rules look at where the download ended up'
	);
});

test('leaves private, non-web and switched-off downloads alone', () => {
	assert.equal(shouldIntercept(item({ incognito: true }), settings()).take, false);
	assert.equal(shouldIntercept(item({ url: 'blob:https://x.test/1234' }), settings()).take, false);
	assert.equal(shouldIntercept(item({ url: 'data:application/zip;base64,AA' }), settings()).take, false);
	assert.equal(shouldIntercept(item(), settings({ intercept: false })).take, false);
	assert.equal(
		shouldIntercept(item({ url: 'https://x.test/big.txt', totalBytes: 5e9 }), settings({ minSizeMB: 0 })).take,
		false,
		'0 MB means size never decides'
	);
});

test('only a download still missing bytes is taken from the browser', () => {
	const paused = { state: 'in_progress', paused: true, totalBytes: 400000, fileSize: 0 };
	assert.equal(stillMissingBytes({ ...paused, bytesReceived: 1000 }), true);
	// Seen in Edge: paused, but every byte already in; it finishes anyway.
	assert.equal(stillMissingBytes({ ...paused, bytesReceived: 400000 }), false);
	assert.equal(stillMissingBytes({ ...paused, totalBytes: 0, bytesReceived: 5000 }), true, 'size unknown');
	assert.equal(stillMissingBytes({ ...paused, paused: false, bytesReceived: 0 }), false, 'pause did not take');
	assert.equal(stillMissingBytes({ ...paused, state: 'complete', bytesReceived: 400000 }), false);
	assert.equal(stillMissingBytes(undefined), false);
});

test('settings are cleaned', () => {
	const s = normalizeSettings({ minSizeMB: '-3', extensions: [' .ZIP', '*.rar', 'zip', ''], neverSites: ['*.Example.com'], intercept: 'yes' });
	assert.equal(s.minSizeMB, DEFAULT_SETTINGS.minSizeMB);
	assert.deepEqual(s.extensions, ['zip', 'rar']);
	assert.deepEqual(s.neverSites, ['example.com']);
	assert.equal(s.intercept, true);
	assert.deepEqual(parseList('a.com, b.com\n\n.c.com  a.com'), ['a.com', 'b.com', 'c.com']);
});

test('sites match themselves and their subdomains only', () => {
	assert.ok(hostMatches('example.com', 'example.com'));
	assert.ok(hostMatches('DL.Example.com', 'example.com'));
	assert.ok(!hostMatches('badexample.com', 'example.com'));
});

test('file types', () => {
	assert.equal(extensionOf('', 'https://x.test/a/B.Tar.GZ?x=1'), 'gz');
	assert.equal(extensionOf('/home/me/file.MKV', ''), 'mkv');
	assert.equal(extensionOf('', 'https://x.test/download'), '');
	assert.equal(extensionOf('', 'https://x.test/.hidden'), '');
});

test('links in text and de-duplication', () => {
	assert.deepEqual(linksInText('see https://a.test/x.zip, and (http://b.test/y).'), [
		'https://a.test/x.zip',
		'http://b.test/y'
	]);
	assert.deepEqual(
		uniqueWebLinks(['https://a.test/1#top', 'https://a.test/1', 'javascript:void(0)', 'mailto:x@y', 'https://a.test/2']),
		['https://a.test/1#top', 'https://a.test/2']
	);
	assert.deepEqual(chunk([1, 2, 3, 4, 5], 2), [[1, 2], [3, 4], [5]]);
});

test('cookies are read once per site and merged', () => {
	const probes = cookieProbes(['https://a.test/x/1', 'https://a.test/y/2', 'https://b.test/', 'https://c.test/'], 2);
	assert.deepEqual(probes, ['https://a.test/x/1', 'https://b.test/']);
	const c = (name, domain, path = '/') => ({ name, domain, path, value: name });
	assert.deepEqual(
		mergeCookies([[c('sid', '.a.test'), c('p', 'a.test', '/x')], [c('sid', '.a.test'), c('sid', 'b.test')]]).map(
			(x) => `${x.name}@${x.domain}${x.path}`
		),
		['sid@.a.test/', 'p@a.test/x', 'sid@b.test/']
	);
});

const MAGNET = 'magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=tool';

test('magnet links are sendable; other schemes are not', () => {
	assert.equal(isMagnetUrl(MAGNET), true);
	assert.equal(isMagnetUrl('MAGNET:?xt=urn:btih:abc'), true, 'scheme is case-insensitive');
	assert.equal(isMagnetUrl('https://x.test/magnet:?x'), false);
	assert.equal(isSendableUrl(MAGNET), true);
	assert.equal(isSendableUrl('https://x.test/a'), true);
	for (const url of ['ftp://x.test/a.zip', 'file:///etc/passwd', 'javascript:alert(1)', 'mailto:a@b.test', 'blob:https://x.test/1', 'data:text/plain,hi', '', undefined]) {
		assert.equal(isSendableUrl(url), false, String(url));
		assert.equal(isMagnetUrl(url), false, String(url));
	}
	assert.deepEqual(
		uniqueWebLinks([MAGNET, 'ftp://x.test/a', 'https://a.test/1', 'javascript:void(0)', MAGNET]),
		[MAGNET, 'https://a.test/1']
	);
	assert.deepEqual(linksInText(`get ${MAGNET}. or https://a.test/x`), [MAGNET, 'https://a.test/x']);
});

test('magnet links do not break request context (no host)', () => {
	// Cookies are read per site; a magnet link has no site, so it adds no probe.
	assert.deepEqual(cookieProbes([MAGNET, 'https://a.test/x/1', MAGNET]), ['https://a.test/x/1']);
	assert.deepEqual(cookieProbes([MAGNET]), []);
	assert.equal(extensionOf('', MAGNET), '', 'no file type from a magnet');
});

test('torrent downloads are always taken over, however small', () => {
	const s = settings({ minSizeMB: 20, neverSites: ['bank.example'] });
	assert.equal(shouldIntercept(item({ url: 'https://x.test/tool.torrent', totalBytes: 2000 }), s).take, true, 'by URL');
	assert.equal(
		shouldIntercept(item({ url: 'https://x.test/get?id=2', filename: 'C:\\Users\\me\\Downloads\\A.TORRENT', totalBytes: 900 }), s).take,
		true,
		'by file name'
	);
	assert.equal(
		shouldIntercept(item({ url: 'https://x.test/get?id=3', mime: 'application/x-bittorrent', totalBytes: 900 }), s).take,
		true,
		'by mime type'
	);
	assert.equal(shouldIntercept(item({ url: 'https://x.test/tool.torrent' }), settings({ intercept: false })).take, false, 'switched off');
	assert.equal(shouldIntercept(item({ url: 'https://x.test/tool.torrent', incognito: true }), s).take, false, 'private window');
	assert.equal(shouldIntercept(item({ url: 'https://dl.bank.example/tool.torrent' }), s).take, false, 'never-site wins');
});

test('magnet links are on by default and the setting is cleaned', () => {
	assert.equal(DEFAULT_SETTINGS.magnetLinks, true);
	assert.equal(normalizeSettings({}).magnetLinks, true);
	assert.equal(normalizeSettings({ magnetLinks: false }).magnetLinks, false);
	assert.equal(normalizeSettings({ magnetLinks: 'no' }).magnetLinks, true, 'malformed value: default');
});

test('extension ids are 32 letters a to p', () => {
	// Whether Chromium agrees is checked by loading the extension (see PLAN.md, "How 2d works").
	assert.match(chromiumId(Buffer.from('test').toString('base64')), /^[a-p]{32}$/);
});
