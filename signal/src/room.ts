import { DurableObject } from 'cloudflare:workers';

import {
  expire,
  join,
  leave,
  MAX_BUFFERED,
  MAX_PEERS,
  relay,
  ROOM_TTL_MS,
  type Peer,
  type RoomStore,
} from './room-rules';

/** What a connection remembers across hibernation. */
interface Attachment {
  messages: number;
}

const attachment = (socket: WebSocket): Attachment =>
  (socket.deserializeAttachment() as Attachment | null) ?? { messages: 0 };

/** One WebSocket as a room [Peer]; its message count survives hibernation. */
function peer(socket: WebSocket): Peer {
  return {
    send: (data) => {
      // A socket can be closing while the room still lists it; it no longer needs news.
      if (socket.readyState === WebSocket.OPEN) socket.send(data);
    },
    close: (code, reason) => {
      socket.close(code, reason);
    },
    countMessage: () => {
      const counted = { ...attachment(socket), messages: attachment(socket).messages + 1 };
      socket.serializeAttachment(counted);
      return counted.messages;
    },
  };
}

/**
 * A signaling room: one Durable Object per room name, holding its devices' WebSockets. It
 * hibernates between messages, so everything it needs lives in storage or on the sockets.
 */
export class Room extends DurableObject {
  private readonly store: RoomStore = {
    buffer: async (message) => {
      const kept = (await this.ctx.storage.get<ArrayBuffer[]>('buffered')) ?? [];
      if (kept.length >= MAX_BUFFERED) return false;
      await this.ctx.storage.put('buffered', [...kept, message]);
      return true;
    },
    takeBuffered: async () => {
      const kept = (await this.ctx.storage.get<ArrayBuffer[]>('buffered')) ?? [];
      if (kept.length > 0) await this.ctx.storage.delete('buffered');
      return kept;
    },
  };

  override async fetch(request: Request): Promise<Response> {
    if (request.headers.get('upgrade')?.toLowerCase() !== 'websocket') {
      return new Response('expected a WebSocket', { status: 426 });
    }
    const [client, server] = Object.values(new WebSocketPair()) as [WebSocket, WebSocket];
    const others = this.ctx.getWebSockets();
    if (others.length >= MAX_PEERS) {
      // Turned away on a plain socket: it never joins the room, so nobody hears of it.
      server.accept();
      await join(peer(server), others.map(peer), this.store);
      return new Response(null, { status: 101, webSocket: client });
    }
    this.ctx.acceptWebSocket(server);
    await join(peer(server), others.map(peer), this.store);
    if ((await this.ctx.storage.getAlarm()) === null) {
      await this.ctx.storage.setAlarm(Date.now() + ROOM_TTL_MS);
    }
    return new Response(null, { status: 101, webSocket: client });
  }

  override async webSocketMessage(socket: WebSocket, message: ArrayBuffer | string) {
    await relay(peer(socket), this.othersThan(socket), message, this.store);
  }

  override webSocketClose(socket: WebSocket, code: number) {
    leave(this.othersThan(socket));
    // Finish the close handshake; a client's own code is echoed back when it's allowed.
    const echo = code === 1000 || (code >= 3000 && code <= 4999) ? code : 1000;
    socket.close(echo, 'bye');
  }

  override async alarm() {
    expire(this.ctx.getWebSockets().map(peer));
    await this.ctx.storage.deleteAll();
  }

  private othersThan(socket: WebSocket): Peer[] {
    return this.ctx
      .getWebSockets()
      .filter((other) => other !== socket)
      .map(peer);
  }
}
