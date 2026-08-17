import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { sessionApi } from '@/shared/lib/api';
import type { Agent, AttentionKind, HistoryItem, SessionInfo } from '@/types';

export function useSessions(backendAddress: string, backendKey = backendAddress || 'embedded', enabled = true) {
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [attention, setAttention] = useState<Record<string, AttentionKind>>({});
  const [nativeAttention, setNativeAttention] = useState<Record<string, AttentionKind>>({});
  const [stateBackendKey, setStateBackendKey] = useState(backendKey);
  const historyMap = useRef(new Map<string, string>());
  const generation = useRef(0);
  const currentBackendKey = useRef(backendKey);
  const removalTimers = useRef(new Map<string, number>());
  const dismissedSessions = useRef(new Set<string>());
  const skipPersistenceForBackend = useRef<string | null>(backendKey);

  if (currentBackendKey.current !== backendKey) {
    currentBackendKey.current = backendKey;
    generation.current += 1;
    skipPersistenceForBackend.current = backendKey;
  }

  useEffect(() => {
    setSessions([]);
    setAttention({});
    setNativeAttention({});
    historyMap.current.clear();
    removalTimers.current.forEach(window.clearTimeout);
    removalTimers.current.clear();
    dismissedSessions.current.clear();
    setActiveId(localStorage.getItem(`akmux.active-session:${backendKey}`));
    setStateBackendKey(backendKey);
    return () => {
      removalTimers.current.forEach(window.clearTimeout);
      removalTimers.current.clear();
    };
  }, [backendKey]);

  useEffect(() => {
    if (skipPersistenceForBackend.current === backendKey) {
      skipPersistenceForBackend.current = null;
      return;
    }
    if (activeId) localStorage.setItem(`akmux.active-session:${backendKey}`, activeId);
    else localStorage.removeItem(`akmux.active-session:${backendKey}`);
  }, [activeId, backendKey]);

  const load = useCallback(async () => {
    if (!enabled) return;
    const requestGeneration = generation.current;
    const listed = await sessionApi.sessions(backendAddress);
    if (requestGeneration !== generation.current) return;
    for (const id of dismissedSessions.current) {
      if (!listed.some(session => session.id === id)) dismissedSessions.current.delete(id);
      else void sessionApi.closeSession(backendAddress, id).catch(() => undefined);
    }
    const incoming = listed.filter(session => session.status !== 'exited' && !dismissedSessions.current.has(session.id));
    setSessions(incoming);
    setActiveId(current => {
      if (!incoming.length) return null;
      return current && incoming.some(session => session.id === current) ? current : incoming[0].id;
    });
  }, [backendAddress, backendKey, enabled]);

  useEffect(() => {
    if (!enabled) return;
    void load().catch(() => undefined);
    const timer = window.setInterval(() => void load().catch(() => undefined), 10_000);
    return () => window.clearInterval(timer);
  }, [enabled, load]);

  const add = (session: SessionInfo, historyKey?: string) => {
    setSessions(current => [...current.filter(item => item.id !== session.id), session]);
    if (historyKey) historyMap.current.set(historyKey, session.id);
    setActiveId(session.id);
    setAttention(current => {
      const next = { ...current };
      delete next[session.id];
      return next;
    });
  };

  const select = (id: string) => {
    setActiveId(id);
    const session = sessions.find(item => item.id === id);
    if (session?.native_session_id) {
      const key = `${session.agent}:${session.native_session_id}`;
      setNativeAttention(current => {
        const next = { ...current };
        delete next[key];
        return next;
      });
    }
    setAttention(current => {
      if (!current[id]) return current;
      const next = { ...current };
      delete next[id];
      return next;
    });
  };

  const create = async (agent: Agent, cwd: string) => {
    const requestGeneration = generation.current;
    const session = await sessionApi.createSession(backendAddress, agent, cwd);
    if (requestGeneration !== generation.current) return;
    add(session);
  };

  const resume = async (item: HistoryItem) => {
    const requestGeneration = generation.current;
    const key = `${item.agent}:${item.id}`;
    setNativeAttention(current => {
      const next = { ...current };
      delete next[key];
      return next;
    });
    const existing = historyMap.current.get(key);
    if (existing && sessions.some(session => session.id === existing)) {
      select(existing);
      return;
    }
    const session = await sessionApi.createSession(backendAddress, item.agent, item.cwd, item.id);
    if (requestGeneration !== generation.current) return;
    add(session, key);
  };

  const remove = (id: string) => {
    const timer = removalTimers.current.get(id);
    if (timer !== undefined) window.clearTimeout(timer);
    removalTimers.current.delete(id);
    setSessions(current => {
      const index = current.findIndex(session => session.id === id);
      const next = current.filter(session => session.id !== id);
      setActiveId(active => {
        if (active !== id) return active;
        return next[Math.min(index, next.length - 1)]?.id || null;
      });
      return next;
    });
    setAttention(current => {
      const next = { ...current };
      delete next[id];
      return next;
    });
  };

  const markAttention = (id: string, kind: AttentionKind) => {
    setAttention(current => ({ ...current, [id]: kind }));
    const session = sessions.find(item => item.id === id);
    if (session?.native_session_id) setNativeAttention(current => ({ ...current, [`${session.agent}:${session.native_session_id}`]: kind }));
  };

  const update = (session: SessionInfo) => {
    if (session.status === 'exited') {
      if (!removalTimers.current.has(session.id)) {
        removalTimers.current.set(session.id, window.setTimeout(() => remove(session.id), 650));
      }
      return;
    }
    setSessions(current => current.map(item => (item.id === session.id ? session : item)));
  };

  const close = async (id: string) => {
    const requestGeneration = generation.current;
    dismissedSessions.current.add(id);
    remove(id);
    try {
      await sessionApi.closeSession(backendAddress, id);
    } catch {
      // Keep the local surface dismissible while a disconnected backend recovers.
    }
    if (requestGeneration !== generation.current) return;
  };

  const visibleSessions = stateBackendKey === backendKey ? sessions : [];
  const visibleActiveId = stateBackendKey === backendKey ? activeId : null;
  const active = useMemo(() => visibleSessions.find(session => session.id === visibleActiveId), [visibleSessions, visibleActiveId]);

  return {
    sessions: visibleSessions,
    active,
    activeId: visibleActiveId,
    setActiveId: select,
    attention: stateBackendKey === backendKey ? attention : {},
    nativeAttention: stateBackendKey === backendKey ? nativeAttention : {},
    create,
    resume,
    update,
    close,
    markAttention,
    load,
  };
}
