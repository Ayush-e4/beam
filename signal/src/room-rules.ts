/**
 * What a signaling room does, apart from Cloudflare's plumbing (see `room.ts`). Two devices
 * meet in a room named after their pairing code and pass each other sealed messages. The room
 * never reads them: they are opaque bytes, sealed with a key the server doesn't have.
 */

/** A room lives this long after its first device joins, then closes. */
export const ROOM_TTL_MS = 10 * 60 * 1000;
/** A phone and a browser: a third device is turned away. */
export const MAX_PEERS = 2;
/** Connection details and certificate hashes fit easily. */
export const MAX_MESSAGE_BYTES = 16 * 1024;
/** More than a pairing ever needs; stops a device using the room as a relay. */
export const MAX_MESSAGES_PER_PEER = 64;
/** Messages kept for a device that hasn't joined yet. */
export const MAX_BUFFERED = 8;

/** WebSocket close codes the room uses (4000–4999 are free for applications). */
export const Close = {
  ROOM_FULL: 4001,
  EXPIRED: 4002,
  TOO_MANY_MESSAGES: 4003,
  TOO_BIG: 4004,
  BINARY_ONLY: 4005,
} as const;

/** What the room itself tells a device, as JSON text frames. Peers' messages are binary. */
export type Notice =
  | { type: 'joined'; peers: number }
  | { type: 'peer-joined' }
  | { type: 'peer-left' }
  | { type: 'buffer-full' };

/** A device's connection, as the room sees it. */
export interface Peer {
  send(data: ArrayBuffer | string): void;
  close(code: number, reason: string): void;
  /** Counts one more message from this device and returns the total so far. */
  countMessage(): number;
}

/** What the room keeps between messages. */
export interface RoomStore {
  /** Keeps [message] for the next device to join; false when the buffer is full. */
  buffer(message: ArrayBuffer): Promise<boolean>;
  /** The kept messages, oldest first, removing them. */
  takeBuffered(): Promise<ArrayBuffer[]>;
}

const notice = (body: Notice): string => JSON.stringify(body);

/**
 * A device joins: everyone already there hears of it, and it gets what was sent before it
 * came. Returns false if the room is full (the device is turned away).
 */
export async function join(newcomer: Peer, others: readonly Peer[], store: RoomStore) {
  if (others.length >= MAX_PEERS) {
    newcomer.close(Close.ROOM_FULL, 'room_full');
    return false;
  }
  for (const other of others) other.send(notice({ type: 'peer-joined' }));
  newcomer.send(notice({ type: 'joined', peers: others.length }));
  for (const message of await store.takeBuffered()) newcomer.send(message);
  return true;
}

/** Passes a device's message to the other one, or keeps it until another device joins. */
export async function relay(
  from: Peer,
  others: readonly Peer[],
  message: ArrayBuffer | string,
  store: RoomStore,
) {
  if (typeof message === 'string') {
    from.close(Close.BINARY_ONLY, 'binary_only');
    return;
  }
  if (message.byteLength > MAX_MESSAGE_BYTES) {
    from.close(Close.TOO_BIG, 'too_big');
    return;
  }
  if (from.countMessage() > MAX_MESSAGES_PER_PEER) {
    from.close(Close.TOO_MANY_MESSAGES, 'too_many_messages');
    return;
  }
  if (others.length > 0) {
    for (const other of others) other.send(message);
  } else if (!(await store.buffer(message))) {
    from.send(notice({ type: 'buffer-full' }));
  }
}

/** A device left; the one still there hears of it. */
export function leave(others: readonly Peer[]) {
  for (const other of others) other.send(notice({ type: 'peer-left' }));
}

/** The room's time is up: every device is disconnected. */
export function expire(everyone: readonly Peer[]) {
  for (const peer of everyone) peer.close(Close.EXPIRED, 'expired');
}
