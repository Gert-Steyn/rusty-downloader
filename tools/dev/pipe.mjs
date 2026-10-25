// Talk to the running app's browser pipe, as the native host will (see PLAN.md, "How 2b works").
// Each argument is one message (JSON); `v` is filled in when missing. Replies are printed.
//   node tools/dev/pipe.mjs '{"type":"ping"}' '{"type":"status"}'
//   node tools/dev/pipe.mjs '{"type":"add","links":["https://example.com/a.zip"],"target":"queue"}'
//   node tools/dev/pipe.mjs --raw 'not json'      (sent exactly as given)
import { connect } from 'node:net';
import { userInfo } from 'node:os';

const args = process.argv.slice(2);
const raw = args[0] === '--raw';
const messages = (raw ? args.slice(1) : args).map((a) => (raw ? a : JSON.stringify({ v: 1, ...JSON.parse(a) })));
const user = (process.env.USERNAME || userInfo().username).replace(/[^A-Za-z0-9._-]/g, '') || 'user';
const name = `\\\\.\\pipe\\rusty-downloader-${user}`;

const socket = connect(name);
socket.on('error', (e) => {
	console.error(`${name}: ${e.message}`);
	process.exit(1);
});
await new Promise((r) => socket.once('connect', r));

let buffer = Buffer.alloc(0);
const replies = [];
socket.on('data', (chunk) => {
	buffer = Buffer.concat([buffer, chunk]);
	while (buffer.length >= 4 && buffer.length >= 4 + buffer.readUInt32LE(0)) {
		const len = buffer.readUInt32LE(0);
		replies.shift()?.(buffer.subarray(4, 4 + len).toString('utf8'));
		buffer = buffer.subarray(4 + len);
	}
});

for (const message of messages) {
	const body = Buffer.from(message, 'utf8');
	const header = Buffer.alloc(4);
	header.writeUInt32LE(body.length);
	const reply = new Promise((r) => replies.push(r));
	socket.write(Buffer.concat([header, body]));
	console.log(await reply);
}
socket.end();
