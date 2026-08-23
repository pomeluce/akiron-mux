import { invoke } from '@tauri-apps/api/core';
import { desktopShell } from '@/features/desktop/desktop-shell';
import type { BackendProfile } from '@/types';

let activeProfile: BackendProfile | null = null;

export function configureDesktopBackend(profile: BackendProfile | null) {
  activeProfile = desktopShell ? profile : null;
}

export function currentDesktopBackend() {
  return activeProfile;
}

export async function desktopBackendRequest(method: string, path: string, body?: unknown) {
  if (!activeProfile) return null;
  return invoke<{ status: number; body: unknown }>('backend_request', {
    request: { profileId: activeProfile.id, method, path, body: body ?? null },
  });
}
