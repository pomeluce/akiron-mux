import { useCallback, useEffect, useMemo, useReducer, useRef } from 'react';
import { sessionApi } from '@/shared/lib/api';
import type { Agent, AttentionKind, HistoryItem, SessionInfo } from '@/types';
import { browserSessionSelectionPersistence, type SessionSelectionPersistence } from './session-selection-persistence';

const ATTENTION_DEDUPLICATION_MS = 2_000;

export interface SessionNotificationAdapter {
  shouldNotify(): boolean;
  notify(session: SessionInfo, kind: AttentionKind): void | Promise<void>;
}

interface UnifiedSessionsOptions {
  backendAddress: string;
  backendKey?: string;
  enabled?: boolean;
  persistence?: SessionSelectionPersistence;
  notifications?: SessionNotificationAdapter;
}

interface UnifiedSessionsSnapshot {
  backendKey: string;
  generation: number;
  hydrated: boolean;
  sessions: SessionInfo[];
  activeId: string | null;
  attention: Record<string, AttentionKind>;
  nativeAttention: Record<string, AttentionKind>;
  focusRequest: { sessionId: string | null; revision: number };
}

type SnapshotAction =
  | { type: 'reset'; backendKey: string; generation: number; restoredActiveId: string | null }
  | { type: 'loaded'; backendKey: string; generation: number; sessions: SessionInfo[] }
  | { type: 'add'; backendKey: string; generation: number; session: SessionInfo }
  | { type: 'select'; backendKey: string; generation: number; sessionId: string; focus: boolean }
  | { type: 'remove'; backendKey: string; generation: number; sessionId: string; focusReplacement: boolean }
  | { type: 'update'; backendKey: string; generation: number; session: SessionInfo }
  | { type: 'attention'; backendKey: string; generation: number; session: SessionInfo; kind: AttentionKind }
  | { type: 'clearNativeAttention'; backendKey: string; generation: number; nativeKey: string };

function initialSnapshot(backendKey: string): UnifiedSessionsSnapshot {
  return {
    backendKey,
    generation: 0,
    hydrated: false,
    sessions: [],
    activeId: null,
    attention: {},
    nativeAttention: {},
    focusRequest: { sessionId: null, revision: 0 },
  };
}

function snapshotMatches(snapshot: UnifiedSessionsSnapshot, action: Exclude<SnapshotAction, { type: 'reset' }>) {
  return snapshot.backendKey === action.backendKey && snapshot.generation === action.generation;
}

function clearSelectedAttention(snapshot: UnifiedSessionsSnapshot, session: SessionInfo) {
  const attention = { ...snapshot.attention };
  delete attention[session.id];
  const nativeAttention = { ...snapshot.nativeAttention };
  if (session.native_session_id) delete nativeAttention[`${session.agent}:${session.native_session_id}`];
  return { attention, nativeAttention };
}

function reduceSnapshot(snapshot: UnifiedSessionsSnapshot, action: SnapshotAction): UnifiedSessionsSnapshot {
  if (action.type === 'reset') {
    return {
      ...initialSnapshot(action.backendKey),
      generation: action.generation,
      activeId: action.restoredActiveId,
    };
  }
  if (!snapshotMatches(snapshot, action)) return snapshot;

  switch (action.type) {
    case 'loaded': {
      const activeId = snapshot.activeId && action.sessions.some(session => session.id === snapshot.activeId) ? snapshot.activeId : action.sessions[0]?.id || null;
      return { ...snapshot, hydrated: true, sessions: action.sessions, activeId };
    }
    case 'add': {
      const sessions = [...snapshot.sessions.filter(item => item.id !== action.session.id), action.session];
      const cleared = clearSelectedAttention(snapshot, action.session);
      return {
        ...snapshot,
        ...cleared,
        hydrated: true,
        sessions,
        activeId: action.session.id,
        focusRequest: { sessionId: action.session.id, revision: snapshot.focusRequest.revision + 1 },
      };
    }
    case 'select': {
      const session = snapshot.sessions.find(item => item.id === action.sessionId);
      if (!session) return snapshot;
      const cleared = clearSelectedAttention(snapshot, session);
      return {
        ...snapshot,
        ...cleared,
        activeId: session.id,
        focusRequest: action.focus ? { sessionId: session.id, revision: snapshot.focusRequest.revision + 1 } : snapshot.focusRequest,
      };
    }
    case 'remove': {
      const index = snapshot.sessions.findIndex(session => session.id === action.sessionId);
      if (index < 0) return snapshot;
      const sessions = snapshot.sessions.filter(session => session.id !== action.sessionId);
      const attention = { ...snapshot.attention };
      delete attention[action.sessionId];
      if (snapshot.activeId !== action.sessionId) return { ...snapshot, sessions, attention };
      const activeId = sessions[Math.min(index, sessions.length - 1)]?.id || null;
      return {
        ...snapshot,
        sessions,
        activeId,
        attention,
        focusRequest:
          action.focusReplacement && activeId ? { sessionId: activeId, revision: snapshot.focusRequest.revision + 1 } : { ...snapshot.focusRequest, sessionId: null },
      };
    }
    case 'update':
      return { ...snapshot, sessions: snapshot.sessions.map(item => (item.id === action.session.id ? action.session : item)) };
    case 'attention': {
      if (snapshot.activeId === action.session.id) return snapshot;
      const attention = { ...snapshot.attention, [action.session.id]: action.kind };
      const nativeAttention = action.session.native_session_id
        ? { ...snapshot.nativeAttention, [`${action.session.agent}:${action.session.native_session_id}`]: action.kind }
        : snapshot.nativeAttention;
      return { ...snapshot, attention, nativeAttention };
    }
    case 'clearNativeAttention': {
      if (!snapshot.nativeAttention[action.nativeKey]) return snapshot;
      const nativeAttention = { ...snapshot.nativeAttention };
      delete nativeAttention[action.nativeKey];
      return { ...snapshot, nativeAttention };
    }
  }
}

export function useSessions({
  backendAddress,
  backendKey = backendAddress || 'embedded',
  enabled = true,
  persistence = browserSessionSelectionPersistence,
  notifications,
}: UnifiedSessionsOptions) {
  const [snapshot, dispatch] = useReducer(reduceSnapshot, backendKey, initialSnapshot);
  const snapshotRef = useRef(snapshot);
  const historyMap = useRef(new Map<string, string>());
  const generation = useRef(0);
  const currentBackendKey = useRef(backendKey);
  const removalTimers = useRef(new Map<string, number>());
  const dismissedSessions = useRef(new Set<string>());
  const recentAttention = useRef(new Map<string, number>());
  const notificationsRef = useRef(notifications);
  notificationsRef.current = notifications;
  snapshotRef.current = snapshot;

  if (currentBackendKey.current !== backendKey) {
    currentBackendKey.current = backendKey;
    generation.current += 1;
  }

  const actionContext = () => ({ backendKey: currentBackendKey.current, generation: generation.current });

  useEffect(() => {
    const context = actionContext();
    historyMap.current.clear();
    removalTimers.current.forEach(window.clearTimeout);
    removalTimers.current.clear();
    dismissedSessions.current.clear();
    recentAttention.current.clear();
    dispatch({ type: 'reset', ...context, restoredActiveId: persistence.read(backendKey) });
    return () => {
      removalTimers.current.forEach(window.clearTimeout);
      removalTimers.current.clear();
    };
  }, [backendKey, persistence]);

  useEffect(() => {
    if (snapshot.backendKey !== backendKey || !snapshot.hydrated) return;
    persistence.write(backendKey, snapshot.activeId);
  }, [backendKey, persistence, snapshot.activeId, snapshot.backendKey, snapshot.hydrated]);

  const load = useCallback(async () => {
    if (!enabled) return;
    const context = actionContext();
    const listed = await sessionApi.sessions(backendAddress);
    if (context.generation !== generation.current || context.backendKey !== currentBackendKey.current) return;
    for (const id of dismissedSessions.current) {
      if (!listed.some(session => session.id === id)) dismissedSessions.current.delete(id);
      else void sessionApi.closeSession(backendAddress, id).catch(() => undefined);
    }
    const sessions = listed.filter(session => session.status !== 'exited' && !dismissedSessions.current.has(session.id));
    dispatch({ type: 'loaded', ...context, sessions });
  }, [backendAddress, backendKey, enabled]);

  useEffect(() => {
    if (!enabled) return;
    void load().catch(() => undefined);
    const timer = window.setInterval(() => void load().catch(() => undefined), 10_000);
    return () => window.clearInterval(timer);
  }, [enabled, load]);

  const select = (sessionId: string) => {
    dispatch({ type: 'select', ...actionContext(), sessionId, focus: true });
  };

  const create = async (agent: Agent, cwd: string) => {
    const context = actionContext();
    const session = await sessionApi.createSession(backendAddress, agent, cwd);
    if (context.generation !== generation.current || context.backendKey !== currentBackendKey.current) return;
    dispatch({ type: 'add', ...context, session });
  };

  const resume = async (item: HistoryItem) => {
    const context = actionContext();
    const nativeKey = `${item.agent}:${item.id}`;
    dispatch({ type: 'clearNativeAttention', ...context, nativeKey });
    const existing = historyMap.current.get(nativeKey);
    if (existing && snapshotRef.current.sessions.some(session => session.id === existing)) {
      select(existing);
      return;
    }
    const session = await sessionApi.createSession(backendAddress, item.agent, item.cwd, item.id);
    if (context.generation !== generation.current || context.backendKey !== currentBackendKey.current) return;
    historyMap.current.set(nativeKey, session.id);
    dispatch({ type: 'add', ...context, session });
  };

  const remove = (sessionId: string, focusReplacement = true) => {
    const timer = removalTimers.current.get(sessionId);
    if (timer !== undefined) window.clearTimeout(timer);
    removalTimers.current.delete(sessionId);
    recentAttention.current.delete(`${sessionId}:input`);
    recentAttention.current.delete(`${sessionId}:completed`);
    dispatch({ type: 'remove', ...actionContext(), sessionId, focusReplacement });
  };

  const update = (session: SessionInfo) => {
    if (session.status === 'exited') {
      if (!removalTimers.current.has(session.id)) {
        removalTimers.current.set(session.id, window.setTimeout(() => remove(session.id), 650));
      }
      return;
    }
    dispatch({ type: 'update', ...actionContext(), session });
  };

  const handleAttention = (session: SessionInfo, kind: AttentionKind) => {
    const current = snapshotRef.current;
    if (current.backendKey !== currentBackendKey.current || !current.sessions.some(item => item.id === session.id)) return;
    const now = performance.now();
    const key = `${session.id}:${kind}`;
    const previous = recentAttention.current.get(key);
    if (previous !== undefined && now - previous < ATTENTION_DEDUPLICATION_MS) return;
    recentAttention.current.set(key, now);
    dispatch({ type: 'attention', ...actionContext(), session, kind });
    const adapter = notificationsRef.current;
    if (adapter?.shouldNotify()) void adapter.notify(session, kind);
  };

  const close = async (sessionId: string) => {
    dismissedSessions.current.add(sessionId);
    remove(sessionId);
    try {
      await sessionApi.closeSession(backendAddress, sessionId);
    } catch {
      // Keep the local surface dismissible while a disconnected backend recovers.
    }
  };

  const visibleSnapshot = snapshot.backendKey === backendKey ? snapshot : { ...initialSnapshot(backendKey), generation: generation.current };
  const active = useMemo(
    () => visibleSnapshot.sessions.find(session => session.id === visibleSnapshot.activeId),
    [visibleSnapshot.sessions, visibleSnapshot.activeId],
  );

  return {
    sessions: visibleSnapshot.sessions,
    active,
    activeId: visibleSnapshot.activeId,
    attention: visibleSnapshot.attention,
    nativeAttention: visibleSnapshot.nativeAttention,
    focusRequest: visibleSnapshot.focusRequest,
    select,
    create,
    resume,
    update,
    close,
    handleAttention,
    load,
  };
}
