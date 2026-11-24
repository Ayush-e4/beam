import { errorResponse } from './responses';
import { route } from './router';

/**
 * Beam signaling service. It only relays connection details between devices and never
 * sees file data. Pairing rooms (Durable Objects) arrive in M3.
 */
export default {
  fetch(request: Request): Response {
    try {
      return route(request);
    } catch (error) {
      console.error('Unhandled signaling error', error);
      return errorResponse(500, 'internal_error', 'Unexpected server error');
    }
  },
} satisfies ExportedHandler;
