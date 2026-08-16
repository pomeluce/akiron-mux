import { invoke } from '@tauri-apps/api/core';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { desktopShell } from '@/features/desktop/desktop-shell';
import { configureDesktopBackend } from './desktop-backend';
import type { BackendHealth, BackendProfile, BackendProfileState } from '@/types';

const fallback: BackendProfileState = {
  profiles: [{ id: 'local', name: 'Local', kind: 'local', address: 'http://127.0.0.1:17321', instanceId: null, hasCredential: false, requiresAuth: false, capabilities: [] }],
  activeProfileId: 'local',
};

export function useBackends() {
  const [state, setState] = useState(fallback);
  const [loading, setLoading] = useState(desktopShell);

  useEffect(() => {
    if (!desktopShell) return;
    void invoke<BackendProfileState>('list_backend_profiles')
      .then(async next => {
        const legacy = localStorage.getItem('akironmux-backend-address');
        if (legacy) {
          try {
            const url = new URL(legacy);
            if (url.protocol === 'http:' && (url.hostname === '127.0.0.1' || url.hostname === '::1' || url.hostname === 'localhost')) {
              const local = next.profiles.find(profile => profile.id === 'local');
              if (local && local.address !== legacy) {
                next = await invoke<BackendProfileState>('save_backend_profile', {
                  profile: { ...local, address: legacy },
                  confirmedInstanceId: null,
                });
              }
            }
          } catch {
            // Invalid legacy values are discarded instead of entering native profile storage.
          }
          localStorage.removeItem('akironmux-backend-address');
        }
        setState(next);
      })
      .finally(() => setLoading(false));
  }, []);

  const active = state.profiles.find(profile => profile.id === state.activeProfileId) || state.profiles[0];
  useEffect(() => configureDesktopBackend(!loading ? active || null : null), [active, loading]);
  const refreshActive = useCallback(async (profileId: string) => {
    const next = await invoke<BackendProfileState>('refresh_backend_profile', { profileId });
    setState(next);
    return next;
  }, []);

  return useMemo(
    () => ({
      state,
      active,
      loading,
      select: async (profileId: string) => {
        const next = await invoke<BackendProfileState>('activate_backend_profile', { profileId });
        setState(next);
      },
      test: (profile: BackendProfile) => invoke<BackendHealth>('test_backend_profile', { profile }),
      save: async (profile: BackendProfile, confirmedInstanceId?: string) => {
        const next = await invoke<BackendProfileState>('save_backend_profile', {
          profile,
          confirmedInstanceId: confirmedInstanceId || null,
        });
        setState(next);
        return next;
      },
      pair: async (profile: BackendProfile, pairingLink: string, confirmedInstanceId?: string) => {
        const next = await invoke<BackendProfileState>('pair_backend_profile', {
          profile,
          pairingLink,
          confirmedInstanceId: confirmedInstanceId || null,
        });
        setState(next);
        return next;
      },
      refreshActive,
      reorder: async (profileIds: string[]) => {
        const next = await invoke<BackendProfileState>('reorder_backend_profiles', { profileIds });
        setState(next);
      },
      remove: async (profileId: string) => {
        const next = await invoke<BackendProfileState>('delete_backend_profile', { profileId });
        setState(next);
        return next;
      },
    }),
    [active, loading, refreshActive, state],
  );
}

export type BackendManager = ReturnType<typeof useBackends>;
