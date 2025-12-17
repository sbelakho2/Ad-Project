type Snapshot<T = unknown> = {
  timestamp: string;
  payload: T;
};

const store = new Map<string, Snapshot>();

export function saveLKG<T>(key: string, payload: T) {
  store.set(key, { timestamp: new Date().toISOString(), payload });
}

export function getLKG<T = unknown>(key: string): Snapshot<T> | null {
  return (store.get(key) as Snapshot<T> | undefined) ?? null;
}
