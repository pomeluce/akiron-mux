import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { sessionApi } from '@/shared/lib/api';
import type { Agent, HistoryItem, SessionInfo } from '@/types';

function isCompletedNormally(session: SessionInfo) {
  return session.status === 'exited' && (session.exit_code === null || session.exit_code === 0) && !session.error;
}

export function useSessions(backendAddress: string) {
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const historyMap = useRef(new Map<string, string>());

  const load = useCallback(async () => {
    const incoming = (await sessionApi.sessions(backendAddress)).filter(session => !isCompletedNormally(session));
    setSessions(incoming);
    setActiveId(current => (current && incoming.some(session => session.id === current) ? current : incoming.at(0)?.id || null));
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
  };

  const create = async (agent: Agent, cwd: string) => {
    const session = await sessionApi.createSession(backendAddress, agent, cwd);
    add(session);
  };

  const resume = async (item: HistoryItem) => {
    const key = `${item.agent}:${item.id}`;
    const existing = historyMap.current.get(key);
    if (existing && sessions.some(session => session.id === existing)) {
      setActiveId(existing);
      return;
    }
    const session = await sessionApi.createSession(backendAddress, item.agent, item.cwd, item.id);
    add(session, key);
  };

  const update = (session: SessionInfo) => {
    if (isCompletedNormally(session)) {
      window.setTimeout(() => {
        setSessions(current => current.filter(item => item.id !== session.id));
        setActiveId(current => (current === session.id ? null : current));
      }, 650);
      return;
    }
    setSessions(current => current.map(item => (item.id === session.id ? session : item)));
  };

  const close = async (id: string) => {
    await sessionApi.closeSession(backendAddress, id);
    setSessions(current => current.filter(session => session.id !== id));
    setActiveId(current => (current === id ? null : current));
  };

  const active = useMemo(() => sessions.find(session => session.id === activeId), [sessions, activeId]);

  return { sessions, active, activeId, setActiveId, create, resume, update, close, load };
}
