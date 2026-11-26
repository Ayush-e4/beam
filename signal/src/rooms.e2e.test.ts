// The real Worker and its rooms (Durable Objects), run locally by Wrangler, joined with real
// WebSockets the way the phone and the browser will.
import { unstable_startWorker } from 'wrangler';

import { Close } from './room-rules';

type Worker = Awaited<ReturnType<typeof unstable_startWorker>>;

let worker: Worker;
let base: string;

beforeAll(async () => {
  worker = await unstable_startWorker({ config: 'wrangler.jsonc', dev: { logLevel: 'none' } });
  const url = await worker.url;
  base = `ws://${url.host}`;
}, 60_000);

afterAll(async () => {
  await worker.dispose();
});

/** A device in a room: everything it has received, and how it was closed. */
interface Device {
  socket: WebSocket;
  received: (ArrayBuffer | string)[];
  closed: Promise<number>;
}

const randomRoom = () =>
  Array.from(crypto.getRandomValues(new Uint8Array(16)), (b) =>
    b.toString(16).padStart(2, '0'),
  ).join('');

async function enter(room: string): Promise<Device> {
  const socket = new WebSocket(`${base}/rooms/${room}`);
  socket.binaryType = 'arraybuffer';
  const received: (ArrayBuffer | string)[] = [];
  socket.addEventListener('message', (event) => {
    received.push(event.data as ArrayBuffer | string);
  });
  const closed = new Promise<number>((resolve) => {
    socket.addEventListener('close', (event) => {
      resolve(event.code);
    });
  });
  await new Promise((resolve, reject) => {
    socket.addEventListener('open', resolve);
    socket.addEventListener('error', reject);
  });
  return { socket, received, closed };
}

/** Waits until [device] has received [count] messages, and returns them. */
async function nextMessages(device: Device, count: number) {
  for (let i = 0; i < 200 && device.received.length < count; i++) {
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  expect(device.received).toHaveLength(count);
  return device.received;
}

const notice = (data: ArrayBuffer | string | undefined) =>
  typeof data === 'string' ? (JSON.parse(data) as unknown) : data;

describe('signaling rooms', () => {
  it('let two devices pass sealed messages both ways', async () => {
    const room = randomRoom();
    const browser = await enter(room);
    expect(notice((await nextMessages(browser, 1))[0])).toEqual({ type: 'joined', peers: 0 });

    const phone = await enter(room);
    expect(notice((await nextMessages(phone, 1))[0])).toEqual({ type: 'joined', peers: 1 });
    expect(notice((await nextMessages(browser, 2))[1])).toEqual({ type: 'peer-joined' });

    phone.socket.send(new Uint8Array([1, 2, 3]));
    expect(new Uint8Array((await nextMessages(browser, 3))[2] as ArrayBuffer)).toEqual(
      new Uint8Array([1, 2, 3]),
    );
    browser.socket.send(new Uint8Array([9]));
    expect(new Uint8Array((await nextMessages(phone, 2))[1] as ArrayBuffer)).toEqual(
      new Uint8Array([9]),
    );

    phone.socket.close(1000);
    expect(notice((await nextMessages(browser, 4))[3])).toEqual({ type: 'peer-left' });
    browser.socket.close(1000);
  });

  it('keep what a device sent before the other joined', async () => {
    const room = randomRoom();
    const phone = await enter(room);
    await nextMessages(phone, 1);
    phone.socket.send(new Uint8Array([7]));
    phone.socket.send(new Uint8Array([8]));
    await new Promise((resolve) => setTimeout(resolve, 100));

    const browser = await enter(room);
    const [joined, first, second] = await nextMessages(browser, 3);
    expect(notice(joined)).toEqual({ type: 'joined', peers: 1 });
    expect(new Uint8Array(first as ArrayBuffer)).toEqual(new Uint8Array([7]));
    expect(new Uint8Array(second as ArrayBuffer)).toEqual(new Uint8Array([8]));
    phone.socket.close(1000);
    browser.socket.close(1000);
  });

  it('turn a third device away without upsetting the two inside', async () => {
    const room = randomRoom();
    const browser = await enter(room);
    const phone = await enter(room);
    await nextMessages(browser, 2);

    const third = await enter(room);
    expect(await third.closed).toBe(Close.ROOM_FULL);

    phone.socket.send(new Uint8Array([5]));
    const seen = await nextMessages(browser, 3);
    expect(new Uint8Array(seen[2] as ArrayBuffer)).toEqual(new Uint8Array([5]));
    phone.socket.close(1000);
    browser.socket.close(1000);
  });

  it('disconnect a device that sends text', async () => {
    const phone = await enter(randomRoom());
    phone.socket.send('not sealed');
    expect(await phone.closed).toBe(Close.BINARY_ONLY);
  });
});
