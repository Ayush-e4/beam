import { PROTOCOL_VERSION, route } from './router';

const request = (path: string, method = 'GET') =>
  new Request(`https://signal.test${path}`, { method });

describe('route', () => {
  it('reports health with the protocol version', async () => {
    const response = route(request('/health'));

    expect(response.status).toBe(200);
    expect(response.headers.get('cache-control')).toBe('no-store');
    await expect(response.json()).resolves.toEqual({ status: 'ok', protocol: PROTOCOL_VERSION });
  });

  it('rejects non-GET health checks', async () => {
    const response = route(request('/health', 'POST'));

    expect(response.status).toBe(405);
    await expect(response.json()).resolves.toMatchObject({ error: { code: 'method_not_allowed' } });
  });

  it('returns a typed 404 for unknown routes', async () => {
    const response = route(request('/nope'));

    expect(response.status).toBe(404);
    await expect(response.json()).resolves.toMatchObject({ error: { code: 'not_found' } });
  });
});
