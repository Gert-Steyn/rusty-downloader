// Turning drag-and-drop payloads into text the backend can pull links from.

const TEXT_FILE = /\.(txt|url|html?|csv|md|json|xml|dlc)$/i;

/** True if the drag carries something we might find links in. */
export function isLinkDrag(dt: DataTransfer | null): boolean {
	if (!dt) return false;
	return dt.types.some((t) => t === 'text/uri-list' || t === 'text/plain' || t === 'text/html' || t === 'Files');
}

/**
 * Everything textual in a drop: URI lists (browser links), the hrefs of dragged
 * HTML (a selection of a web page), plain text, and the contents of dropped
 * text files such as link lists.
 */
export async function textFromDrop(dt: DataTransfer): Promise<string> {
	const parts: string[] = [];

	const uriList = dt.getData('text/uri-list');
	if (uriList) parts.push(uriList.split(/\r?\n/).filter((l) => !l.startsWith('#')).join('\n'));

	const html = dt.getData('text/html');
	if (html) {
		const doc = new DOMParser().parseFromString(html, 'text/html');
		// getAttribute, not .href: relative links would resolve against this app.
		for (const a of doc.querySelectorAll('a[href]')) parts.push(a.getAttribute('href') ?? '');
	}

	const plain = dt.getData('text/plain');
	if (plain) parts.push(plain);

	for (const file of Array.from(dt.files)) {
		if (TEXT_FILE.test(file.name) || file.type.startsWith('text/')) parts.push(await file.text());
	}
	return parts.join('\n');
}
