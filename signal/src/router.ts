import type { Room } from './room';
import { errorResponse, json } from './responses';

/** Protocol version served by this signaling deployment (matches beam-core). */
export const PROTOCOL_VERSION = '1.0';

/** What the Worker is bound to (see `wrangler.jsonc`). */
export interface Env {
  ROOMS: DurableObjectNamespace<Room>;
}

/** A room name: 16 bytes as lowercase hex, derived from a pairing code (see protocol.md). */
const ROOM_PATH = /^\/rooms\/([0-9a-f]{32})$/;

/** Routes a request to its handler. Unknown routes return a typed 404. */
export async function route(request: Request, env: Env): Promise<Response> {
  const { pathname } = new URL(request.url);

  if (pathname === '/health') {
    if (request.method !== 'GET') {
      return errorResponse(405, 'method_not_allowed', 'Use GET for /health');
    }
    return json({ status: 'ok', protocol: PROTOCOL_VERSION });
  }

  if (pathname.startsWith('/rooms/')) {
    const name = ROOM_PATH.exec(pathname)?.[1];
    if (name === undefined) {
      return errorResponse(400, 'bad_room', 'A room is named by 32 lowercase hex characters');
    }
    if (request.method !== 'GET' || request.headers.get('upgrade')?.toLowerCase() !== 'websocket') {
      return errorResponse(426, 'websocket_required', 'Join a room with a WebSocket');
    }
    return env.ROOMS.get(env.ROOMS.idFromName(name)).fetch(request);
  }

  return errorResponse(404, 'not_found', `No route for ${pathname}`);
}
