import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useEffect, useRef } from 'react';
import type { ClientPreferences, SessionInfo } from '@/types';
import { desktopShell } from './desktop-shell';

export function useDesktopTray(preferences: ClientPreferences, sessions: SessionInfo[], onSelect: (id: string) => void) {
  const selectRef = useRef(onSelect);
  selectRef.current = onSelect;

  useEffect(() => {
    if (!desktopShell) return;
    void invoke('sync_tray_state', {
      closeToTray: preferences.closeBehavior === 'tray',
      locale: preferences.locale,
      sessions: sessions.map(session => ({ id: session.id, title: session.title, agent: session.agent })),
    }).catch(() => undefined);
  }, [preferences.closeBehavior, preferences.locale, sessions]);

  useEffect(() => {
    if (!desktopShell) return;
    const unlisten = listen<string>('tray-open-session', event => selectRef.current(event.payload));
    return () => {
      void unlisten.then(dispose => dispose());
    };
  }, []);
}
