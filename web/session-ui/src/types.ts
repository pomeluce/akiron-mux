export type Agent = 'claude' | 'codex';
export type SessionStatus = 'starting' | 'running' | 'exited' | 'error';
export type SortMode = 'priority' | 'recent' | 'manual';
export type ThemeMode = 'light' | 'dark' | 'system';
export type Locale = 'en' | 'zh-CN';

export interface SessionInfo {
  id: string;
  agent: Agent;
  title: string;
  cwd: string;
  status: SessionStatus;
  created_at_ms: number;
  exit_code: number | null;
  error: string | null;
  native_session_id: string | null;
}

export interface HistoryItem {
  id: string;
  agent: Agent;
  title: string;
  cwd: string;
  start_time: string;
  end_time: string | null;
  file_mtime: string;
  message_count: number;
}

export interface Project {
  id: string;
  name: string;
  path: string;
  pinned: boolean;
  sort_order: number;
}

export interface DirectoryGroup {
  path: string;
  available: boolean;
  items: HistoryItem[];
}

export interface WorkspaceResponse {
  general_root: string;
  projects: Array<{ project: Project; history: HistoryItem[] }>;
  general: DirectoryGroup[];
  other: DirectoryGroup[];
}

export interface SettingsResponse {
  general_root: string;
  projects: Project[];
  other_directories: Array<{ path: string; pinned: boolean; last_opened_ms: number }>;
  project_sort: SortMode;
  general_sort: SortMode;
  other_sort: SortMode;
}

export interface DirectoryListing {
  path: string;
  parent: string | null;
  home: string | null;
  entries: Array<{ name: string; path: string }>;
}

export interface ClientPreferences {
  locale: Locale;
  theme: ThemeMode;
  acrylic: boolean;
  acrylicStrength: number;
  backendAddress: string;
}

export const emptyWorkspace: WorkspaceResponse = {
  general_root: '',
  projects: [],
  general: [],
  other: [],
};

export const emptySettings: SettingsResponse = {
  general_root: '',
  projects: [],
  other_directories: [],
  project_sort: 'priority',
  general_sort: 'recent',
  other_sort: 'recent',
};
