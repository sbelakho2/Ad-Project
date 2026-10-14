import type { SdkEvent } from './types';

type Handler = (payload?: any) => void;

class Emitter {
  private handlers: Map<SdkEvent, Set<Handler>> = new Map();

  on(event: SdkEvent, handler: Handler) {
    if (!this.handlers.has(event)) this.handlers.set(event, new Set());
    this.handlers.get(event)!.add(handler);
    return () => this.off(event, handler);
  }

  off(event: SdkEvent, handler: Handler) {
    this.handlers.get(event)?.delete(handler);
  }

  emit(event: SdkEvent, payload?: any) {
    this.handlers.get(event)?.forEach((h) => {
      try {
        h(payload);
      } catch {
        // ignore listener errors
      }
    });
  }
}

export const emitter = new Emitter();
