import type { BackendProfile } from '@/types';

export function configureDesktopBackend(_profile: BackendProfile | null) {}

export function currentDesktopBackend(): BackendProfile | null {
  return null;
}

export async function desktopBackendRequest(_method: string, _path: string, _body?: unknown): Promise<{ status: number; body: unknown } | null> {
  return null;
}
