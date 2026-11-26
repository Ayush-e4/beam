import { PROTOCOL_VERSION, route, type Env } from './router';

const request = (path: string, method = 'GET', headers: Record<string, string> = {}) =>
  new Request(`https://signal.test${path}`, { method, headers });

/** Rooms that answer with the name they were asked for. */
const env = {
  ROOMS: {
    idFromName: (name: string) => name,
    get: (name: string) => ({
      fetch: () => Promise.resolve(new Response(`room ${name}`)),
    }),
  },
} as unknown as Env;

const ROOM = '0123456789abcdef0123456789abcdef';

describe('route', () => {
  it('reports health with the protocol version', async () => {
    const response = await route(request('/health'), env);

    expect(response.status).toBe(200);
    expect(response.headers.get('cache-control')).toBe('no-store');
    await expect(response.json()).resolves.toEqual({ status: 'ok', protocol: PROTOCOL_VERSION });
  });

  it('rejects non-GET health checks', async () => {
    const response = await route(request('/health', 'POST'), env);

    expect(response.status).toBe(405);
    await expect(response.json()).resolves.toMatchObject({ error: { code: 'method_not_allowed' } });
  });

  it('sends a WebSocket to the room it names', async () => {
    const response = await route(request(`/rooms/${ROOM}`, 'GET', { upgrade: 'websocket' }), env);

    await expect(response.text()).resolves.toBe(`room ${ROOM}`);
  });

  it('rejects room names that are not 32 lowercase hex characters', async () => {
    for (const name of ['abc', ROOM.toUpperCase(), `${ROOM}0`, `${ROOM.slice(1)}g`]) {
      const response = await route(request(`/rooms/${name}`, 'GET', { upgrade: 'websocket' }), env);

      expect(response.status).toBe(400);
      await expect(response.json()).resolves.toMatchObject({ error: { code: 'bad_room' } });
    }
  });

  it('only lets WebSockets into a room', async () => {
    const response = await route(request(`/rooms/${ROOM}`), env);

    expect(response.status).toBe(426);
    await expect(response.json()).resolves.toMatchObject({ error: { code: 'websocket_required' } });
  });

  it('returns a typed 404 for unknown routes', async () => {
    const response = await route(request('/nope'), env);

    expect(response.status).toBe(404);
    await expect(response.json()).resolves.toMatchObject({ error: { code: 'not_found' } });
  });
});
