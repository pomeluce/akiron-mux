import { useCallback, useEffect, useState } from 'react';
import { sessionApi } from '@/shared/lib/api';
import { emptySettings, emptyWorkspace, type SettingsResponse, type WorkspaceResponse } from '@/types';

export function useWorkspaces(backendAddress: string) {
  const [workspace, setWorkspace] = useState<WorkspaceResponse>(emptyWorkspace);
  const [settings, setSettings] = useState<SettingsResponse>(emptySettings);
  const [connected, setConnected] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [nextWorkspace, nextSettings] = await Promise.all([sessionApi.workspace(backendAddress), sessionApi.settings(backendAddress)]);
      setWorkspace(nextWorkspace);
      setSettings(nextSettings);
      setConnected(true);
      setError(null);
    } catch (cause) {
      setConnected(false);
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }, [backendAddress]);

  useEffect(() => {
    void load();
    const timer = window.setInterval(() => void load(), 10_000);
    return () => window.clearInterval(timer);
  }, [load]);

  const refresh = async () => {
    const next = await sessionApi.refreshHistory(backendAddress);
    setWorkspace(next);
    setConnected(true);
  };

  return { workspace, settings, connected, error, load, refresh, setWorkspace, setSettings };
}
