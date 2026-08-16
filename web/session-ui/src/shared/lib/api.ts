import type { Agent, DirectoryListing, Project, SessionDetails, SessionInfo, SettingsResponse, SortMode, WorkspaceResponse } from '@/types';

function normalizedBase(address: string) {
  return address.trim().replace(/\/$/, '');
}

export function apiUrl(address: string, path: string) {
  const base = normalizedBase(address);
  return base ? `${base}${path}` : path;
}

export function websocketUrl(address: string, path: string) {
  const base = normalizedBase(address);
  const origin = base || window.location.origin;
  const url = new URL(path, origin);
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  return url.toString();
}

export async function request<T>(address: string, path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(apiUrl(address, path), {
    ...init,
    headers: { 'Content-Type': 'application/json', ...(init?.headers ?? {}) },
  });
  if (!response.ok) {
    const body = (await response.json().catch(() => ({}))) as { error?: string };
    throw new Error(body.error || response.statusText);
  }
  return response.status === 204 ? (undefined as T) : ((await response.json()) as T);
}

export const sessionApi = {
  workspace: (address: string, query = '') => request<WorkspaceResponse>(address, `/api/workspaces${query ? `?q=${encodeURIComponent(query)}` : ''}`),
  refreshHistory: (address: string) => request<WorkspaceResponse>(address, '/api/history/refresh', { method: 'POST' }),
  settings: (address: string) => request<SettingsResponse>(address, '/api/settings'),
  updateSettings: (address: string, patch: Partial<Pick<SettingsResponse, 'general_root' | 'project_sort' | 'general_sort' | 'other_sort'>>) =>
    request<SettingsResponse>(address, '/api/settings', { method: 'PATCH', body: JSON.stringify(patch) }),
  sessions: (address: string) => request<SessionInfo[]>(address, '/api/sessions'),
  sessionDetails: (address: string, id: string) => request<SessionDetails>(address, `/api/sessions/${encodeURIComponent(id)}/details`),
  createSession: (address: string, agent: Agent, cwd: string, resumeId?: string) =>
    request<SessionInfo>(address, '/api/sessions', {
      method: 'POST',
      body: JSON.stringify({ agent, title: '', cwd, resume_id: resumeId, rows: 36, cols: 120 }),
    }),
  restartSession: (address: string, id: string) => request<void>(address, `/api/sessions/${encodeURIComponent(id)}/restart`, { method: 'POST' }),
  closeSession: (address: string, id: string) => request<void>(address, `/api/sessions/${encodeURIComponent(id)}`, { method: 'DELETE' }),
  directories: (address: string, path: string, showHidden: boolean) =>
    request<DirectoryListing>(address, `/api/directories${path ? `?path=${encodeURIComponent(path)}&show_hidden=${showHidden}` : `?show_hidden=${showHidden}`}`),
  createDirectory: (address: string, parent: string, name: string) => request(address, '/api/directories', { method: 'POST', body: JSON.stringify({ parent, name }) }),
  createProject: (address: string, path: string, name: string) => request<Project>(address, '/api/projects', { method: 'POST', body: JSON.stringify({ path, name }) }),
  updateProject: (address: string, id: string, patch: Partial<Pick<Project, 'name' | 'path' | 'pinned'>>) =>
    request<Project>(address, `/api/projects/${encodeURIComponent(id)}`, { method: 'PATCH', body: JSON.stringify(patch) }),
  deleteProject: (address: string, id: string) => request<void>(address, `/api/projects/${encodeURIComponent(id)}`, { method: 'DELETE' }),
  updateSort: (address: string, section: 'projects' | 'general' | 'other', mode: SortMode) => {
    const key = section === 'projects' ? 'project_sort' : `${section}_sort`;
    return request<SettingsResponse>(address, '/api/settings', { method: 'PATCH', body: JSON.stringify({ [key]: mode }) });
  },
  updateDirectorySort: (address: string, path: string, mode: SortMode) =>
    request<SettingsResponse>(address, '/api/settings', { method: 'PATCH', body: JSON.stringify({ directory_sort: { path, mode } }) }),
  reorder: (address: string, kind: 'projects' | 'directories' | 'sessions', scope: string, ids: string[]) =>
    request<SettingsResponse>(address, '/api/reorder', { method: 'POST', body: JSON.stringify({ kind, scope, ids }) }),
};
