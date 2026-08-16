import { useCallback, useEffect, useRef, useState } from 'react';
import { sessionApi } from '@/shared/lib/api';
import { emptySettings, emptyWorkspace, type SettingsResponse, type WorkspaceResponse } from '@/types';

function normalizeSettings(settings: SettingsResponse): SettingsResponse {
  return {
    ...emptySettings,
    ...settings,
    other_directories: (settings.other_directories || []).map((directory, index) => ({
      ...directory,
      sort_order: directory.sort_order ?? index,
    })),
    directory_sort: settings.directory_sort || {},
    session_order: settings.session_order || {},
  };
}

export function useWorkspaces(backendAddress: string, backendKey = backendAddress || 'embedded', enabled = true) {
  const [workspace, setWorkspace] = useState<WorkspaceResponse>(emptyWorkspace);
  const [settings, setSettings] = useState<SettingsResponse>(emptySettings);
  const [connected, setConnected] = useState(false);
  const [error, setError] = useState(false);
  const [stateBackendKey, setStateBackendKey] = useState(backendKey);
  const generation = useRef(0);
  const currentBackendKey = useRef(backendKey);

  if (currentBackendKey.current !== backendKey) {
    currentBackendKey.current = backendKey;
    generation.current += 1;
  }

  const load = useCallback(async () => {
    if (!enabled) return;
    const requestGeneration = generation.current;
    try {
      const [nextWorkspace, nextSettings] = await Promise.all([sessionApi.workspace(backendAddress), sessionApi.settings(backendAddress)]);
      if (requestGeneration !== generation.current) return;
      setWorkspace(nextWorkspace);
      setSettings(normalizeSettings(nextSettings));
      setConnected(true);
      setError(false);
    } catch {
      if (requestGeneration !== generation.current) return;
      setConnected(false);
      setError(true);
    }
  }, [backendAddress, enabled]);

  useEffect(() => {
    setWorkspace(emptyWorkspace);
    setSettings(emptySettings);
    setConnected(false);
    setStateBackendKey(backendKey);
    if (!enabled) return;
    void load();
    const timer = window.setInterval(() => void load(), 10_000);
    return () => window.clearInterval(timer);
  }, [backendKey, enabled, load]);

  const refresh = async () => {
    const requestGeneration = generation.current;
    const next = await sessionApi.refreshHistory(backendAddress);
    if (requestGeneration !== generation.current) return;
    setWorkspace(next);
    setConnected(true);
  };

  const current = stateBackendKey === backendKey;
  return {
    workspace: current ? workspace : emptyWorkspace,
    settings: current ? settings : emptySettings,
    connected: current && connected,
    error: current && error,
    load,
    refresh,
    setWorkspace,
    setSettings,
  };
}
