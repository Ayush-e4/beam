import { errorResponse } from './responses';
import { route, type Env } from './router';

export { Room } from './room';

/**
 * Beam signaling service. It only relays sealed connection details between devices, in rooms
 * (Durable Objects) named after a pairing code, and never sees file data.
 */
export default {
  async fetch(request, env): Promise<Response> {
    try {
      return await route(request, env);
    } catch (error) {
      console.error('Unhandled signaling error', error);
      return errorResponse(500, 'internal_error', 'Unexpected server error');
    }
  },
} satisfies ExportedHandler<Env>;
