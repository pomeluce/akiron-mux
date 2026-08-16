import { useCallback, useEffect, useState } from 'react';
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

export function useWorkspaces(backendAddress: string) {
  const [workspace, setWorkspace] = useState<WorkspaceResponse>(emptyWorkspace);
  const [settings, setSettings] = useState<SettingsResponse>(emptySettings);
  const [connected, setConnected] = useState(false);
  const [error, setError] = useState(false);

  const load = useCallback(async () => {
    try {
      const [nextWorkspace, nextSettings] = await Promise.all([sessionApi.workspace(backendAddress), sessionApi.settings(backendAddress)]);
      setWorkspace(nextWorkspace);
      setSettings(normalizeSettings(nextSettings));
      setConnected(true);
      setError(false);
    } catch {
      setConnected(false);
      setError(true);
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
