export interface SessionSelectionPersistence {
  read(backendKey: string): string | null;
  write(backendKey: string, sessionId: string | null): void;
}

export const browserSessionSelectionPersistence: SessionSelectionPersistence = {
  read: backendKey => localStorage.getItem(`akmux.active-session:${backendKey}`),
  write: (backendKey, sessionId) => {
    const key = `akmux.active-session:${backendKey}`;
    if (sessionId) localStorage.setItem(key, sessionId);
    else localStorage.removeItem(key);
  },
};
