import { errorResponse, json } from './responses';

/** Protocol version served by this signaling deployment (matches beam-core). */
export const PROTOCOL_VERSION = '1.0';

/** Routes a request to its handler. Unknown routes return a typed 404. */
export function route(request: Request): Response {
  const { pathname } = new URL(request.url);

  if (pathname === '/health') {
    if (request.method !== 'GET') {
      return errorResponse(405, 'method_not_allowed', 'Use GET for /health');
    }
    return json({ status: 'ok', protocol: PROTOCOL_VERSION });
  }

  return errorResponse(404, 'not_found', `No route for ${pathname}`);
}
