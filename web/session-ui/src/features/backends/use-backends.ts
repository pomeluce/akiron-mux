import { invoke } from '@tauri-apps/api/core';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { desktopShell } from '@/features/desktop/desktop-shell';
import type { BackendHealth, BackendIdentityConfirmation, BackendLifecycleOutcome, BackendProfile, BackendProfileState } from '@/types';
import { applyBackendProfileIntent, BackendProfileRefreshLoop } from './backend-profile-lifecycle';
import { configureDesktopBackend } from './desktop-backend';

const fallback: BackendProfileState = {
  profiles: [{ id: 'local', name: 'Local', kind: 'local', address: 'http://127.0.0.1:17321', instanceId: null, hasCredential: false, requiresAuth: false, capabilities: [] }],
  activeProfileId: 'local',
};

export function useBackends() {
  const [state, setState] = useState(fallback);
  const [loading, setLoading] = useState(desktopShell);
  const [identityConfirmation, setIdentityConfirmation] = useState<BackendIdentityConfirmation | null>(null);
  const [connectionStatus, setConnectionStatus] = useState<'ready' | 'offline' | 'authenticationRequired'>('ready');
  const [refreshRevision, setRefreshRevision] = useState(0);
  const refreshLoop = useRef<BackendProfileRefreshLoop | null>(null);
  if (!refreshLoop.current) refreshLoop.current = new BackendProfileRefreshLoop();

  const publish = useCallback((outcome: BackendLifecycleOutcome) => {
    setState(outcome.state);
    if (outcome.type === 'identityConfirmationRequired') {
      refreshLoop.current?.stop();
      setIdentityConfirmation({
        challengeId: outcome.challengeId,
        profileId: outcome.profileId,
        observedInstanceId: outcome.observedInstanceId,
      });
      return;
    }
    setIdentityConfirmation(null);
    setConnectionStatus(outcome.type === 'offline' ? 'offline' : outcome.type === 'authenticationRequired' ? 'authenticationRequired' : 'ready');
  }, []);

  const apply = useCallback(
    async (intent: Parameters<typeof applyBackendProfileIntent>[0]) => {
      const outcome = await applyBackendProfileIntent(intent);
      publish(outcome);
      return outcome;
    },
    [publish],
  );

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
                const outcome = await applyBackendProfileIntent({ type: 'save', profile: { ...local, address: legacy } });
                next = outcome.state;
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

  useEffect(() => {
    const loop = refreshLoop.current;
    if (!loop) return;
    if (!desktopShell || loading || active.kind !== 'remote' || active.requiresAuth) {
      loop.stop();
      return;
    }
    loop.start(active.id, publish);
    return () => loop.stop();
  }, [active.id, active.instanceId, active.kind, active.requiresAuth, loading, publish, refreshRevision]);

  useEffect(() => () => refreshLoop.current?.stop(), []);

  return useMemo(
    () => ({
      state,
      active,
      loading,
      connectionStatus,
      identityConfirmation,
      select: (profileId: string) => apply({ type: 'select', profileId }),
      test: (profile: BackendProfile) => invoke<BackendHealth>('test_backend_profile', { profile }),
      save: (profile: BackendProfile, pairingLink = '') => apply({ type: 'save', profile, pairingLink }),
      confirmIdentity: async (challengeId: string) => {
        const outcome = await apply({ type: 'confirmIdentity', challengeId });
        setRefreshRevision(current => current + 1);
        return outcome;
      },
      cancelIdentity: async (challengeId: string) => {
        const outcome = await applyBackendProfileIntent({ type: 'cancelIdentity', challengeId });
        setIdentityConfirmation(current => (current?.challengeId === challengeId ? null : current));
        setRefreshRevision(current => current + 1);
        return outcome;
      },
      reorder: async (profileIds: string[]) => {
        await apply({ type: 'reorder', profileIds });
      },
      remove: (profileId: string) => apply({ type: 'delete', profileId }),
    }),
    [active, apply, connectionStatus, identityConfirmation, loading, state],
  );
}

export type BackendManager = ReturnType<typeof useBackends>;
