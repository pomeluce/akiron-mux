import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { sessionApi } from '@/shared/lib/api';
import type { Agent, AttentionKind, HistoryItem, SessionInfo } from '@/types';

function isCompletedNormally(session: SessionInfo) {
  return session.status === 'exited' && (session.exit_code === null || session.exit_code === 0) && !session.error;
}

export function useSessions(backendAddress: string) {
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [attention, setAttention] = useState<Record<string, AttentionKind>>({});
  const [nativeAttention, setNativeAttention] = useState<Record<string, AttentionKind>>({});
  const historyMap = useRef(new Map<string, string>());

  const load = useCallback(async () => {
    const incoming = (await sessionApi.sessions(backendAddress)).filter(session => !isCompletedNormally(session));
    setSessions(current => {
      // Keep the mounted terminal surface during a transient reconnect. An empty
      // successful response is not enough evidence that running PTYs disappeared.
      if (!incoming.length && current.length) return current;
      return incoming;
    });
    setActiveId(current => {
      if (!incoming.length) return current;
      return current && incoming.some(session => session.id === current) ? current : incoming[0].id;
    });
  }, [backendAddress]);

  useEffect(() => {
    void load().catch(() => undefined);
    const timer = window.setInterval(() => void load().catch(() => undefined), 10_000);
    return () => window.clearInterval(timer);
  }, [load]);

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
    const session = await sessionApi.createSession(backendAddress, agent, cwd);
    add(session);
  };

  const resume = async (item: HistoryItem) => {
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
    add(session, key);
  };

  const remove = (id: string) => {
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
      setAttention(current => ({ ...current, [session.id]: 'exited' }));
      if (session.native_session_id) setNativeAttention(current => ({ ...current, [`${session.agent}:${session.native_session_id}`]: 'exited' }));
    }
    if (isCompletedNormally(session)) {
      window.setTimeout(() => remove(session.id), 650);
      return;
    }
    setSessions(current => current.map(item => (item.id === session.id ? session : item)));
  };

  const close = async (id: string) => {
    await sessionApi.closeSession(backendAddress, id);
    remove(id);
  };

  const active = useMemo(() => sessions.find(session => session.id === activeId), [sessions, activeId]);

  return { sessions, active, activeId, setActiveId: select, attention, nativeAttention, create, resume, update, close, markAttention, load };
}
