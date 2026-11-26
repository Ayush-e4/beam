import {
  Close,
  expire,
  join,
  leave,
  MAX_BUFFERED,
  MAX_MESSAGE_BYTES,
  MAX_MESSAGES_PER_PEER,
  relay,
  type Peer,
  type RoomStore,
} from './room-rules';

class FakePeer implements Peer {
  readonly received: (ArrayBuffer | string)[] = [];
  closed: { code: number; reason: string } | undefined;
  private messages = 0;

  send(data: ArrayBuffer | string) {
    this.received.push(data);
  }

  close(code: number, reason: string) {
    this.closed = { code, reason };
  }

  countMessage() {
    this.messages += 1;
    return this.messages;
  }

  notices(): unknown[] {
    return this.received
      .filter((data) => typeof data === 'string')
      .map((text) => JSON.parse(text) as unknown);
  }

  bytes() {
    return this.received.filter((data) => typeof data !== 'string');
  }
}

class MemoryStore implements RoomStore {
  kept: ArrayBuffer[] = [];

  buffer(message: ArrayBuffer) {
    if (this.kept.length >= MAX_BUFFERED) return Promise.resolve(false);
    this.kept.push(message);
    return Promise.resolve(true);
  }

  takeBuffered() {
    const kept = this.kept;
    this.kept = [];
    return Promise.resolve(kept);
  }
}

const bytes = (...values: number[]) => new Uint8Array(values).buffer;

describe('a signaling room', () => {
  it('tells each device who is there', async () => {
    const store = new MemoryStore();
    const browser = new FakePeer();
    const phone = new FakePeer();

    expect(await join(browser, [], store)).toBe(true);
    expect(await join(phone, [browser], store)).toBe(true);

    expect(browser.notices()).toEqual([{ type: 'joined', peers: 0 }, { type: 'peer-joined' }]);
    expect(phone.notices()).toEqual([{ type: 'joined', peers: 1 }]);
  });

  it('turns a third device away', async () => {
    const third = new FakePeer();

    expect(await join(third, [new FakePeer(), new FakePeer()], new MemoryStore())).toBe(false);
    expect(third.closed).toEqual({ code: Close.ROOM_FULL, reason: 'room_full' });
  });

  it('passes messages to the other device unchanged', async () => {
    const browser = new FakePeer();
    const phone = new FakePeer();

    await relay(phone, [browser], bytes(1, 2, 3), new MemoryStore());

    expect(browser.bytes()).toEqual([bytes(1, 2, 3)]);
    expect(phone.received).toEqual([]);
  });

  it('keeps messages for a device that has not joined yet', async () => {
    const store = new MemoryStore();
    const phone = new FakePeer();
    await relay(phone, [], bytes(1), store);
    await relay(phone, [], bytes(2), store);

    const browser = new FakePeer();
    await join(browser, [phone], store);

    expect(browser.bytes()).toEqual([bytes(1), bytes(2)]);
    expect(store.kept).toEqual([]);
  });

  it('says when it can keep no more', async () => {
    const store = new MemoryStore();
    const phone = new FakePeer();
    for (let i = 0; i <= MAX_BUFFERED; i++) await relay(phone, [], bytes(i), store);

    expect(store.kept).toHaveLength(MAX_BUFFERED);
    expect(phone.notices()).toEqual([{ type: 'buffer-full' }]);
  });

  it('only carries binary messages', async () => {
    const phone = new FakePeer();
    const browser = new FakePeer();

    await relay(phone, [browser], 'hello', new MemoryStore());

    expect(phone.closed?.code).toBe(Close.BINARY_ONLY);
    expect(browser.received).toEqual([]);
  });

  it('refuses messages that are too big', async () => {
    const phone = new FakePeer();
    const browser = new FakePeer();

    await relay(phone, [browser], new ArrayBuffer(MAX_MESSAGE_BYTES + 1), new MemoryStore());

    expect(phone.closed?.code).toBe(Close.TOO_BIG);
    expect(browser.received).toEqual([]);
  });

  it('disconnects a device that sends too many messages', async () => {
    const phone = new FakePeer();
    const browser = new FakePeer();
    const store = new MemoryStore();
    for (let i = 0; i <= MAX_MESSAGES_PER_PEER; i++) await relay(phone, [browser], bytes(1), store);

    expect(browser.bytes()).toHaveLength(MAX_MESSAGES_PER_PEER);
    expect(phone.closed?.code).toBe(Close.TOO_MANY_MESSAGES);
  });

  it('tells the device left behind', () => {
    const browser = new FakePeer();

    leave([browser]);

    expect(browser.notices()).toEqual([{ type: 'peer-left' }]);
  });

  it('disconnects everyone when it expires', () => {
    const everyone = [new FakePeer(), new FakePeer()];

    expire(everyone);

    for (const peer of everyone) expect(peer.closed?.code).toBe(Close.EXPIRED);
  });
});
