import { Check, PanelLeft, Server } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { desktopShell } from '@/features/desktop/desktop-shell';
import { useDesktopTray } from '@/features/desktop/use-desktop-tray';
import { useBackends } from '@/features/backends/use-backends';
import { WindowControls, toggleDesktopMaximize } from '@/features/desktop/window-controls';
import { SettingsDialog } from '@/features/preferences/settings-dialog';
import { usePreferences } from '@/features/preferences/use-preferences';
import { useWorkspaceIcons } from '@/features/preferences/use-workspace-icons';
import { SearchDialog } from '@/features/sessions/search-dialog';
import { SessionDialog } from '@/features/sessions/session-dialog';
import { notifySession } from '@/features/sessions/session-notifications';
import { useSessions } from '@/features/sessions/use-sessions';
import { WorkspaceShell } from '@/features/sessions/workspace-shell';
import { AppSidebar } from '@/features/workspaces/app-sidebar';
import { IconDialog } from '@/features/workspaces/icon-dialog';
import { ProjectDialog } from '@/features/workspaces/project-dialog';
import { useWorkspaces } from '@/features/workspaces/use-workspaces';
import { ConfirmDialog } from '@/shared/components/confirm-dialog';
import { sessionApi } from '@/shared/lib/api';
import { translate } from '@/shared/lib/i18n';
import { basename } from '@/shared/lib/utils';
import type { WorkspaceIconName } from '@/shared/components/workspace-icon';
import { Button } from '@/shared/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/shared/ui/dropdown-menu';
import { Tooltip, TooltipProvider } from '@/shared/ui/tooltip';
import type { AttentionKind, BackendProfile, ClientPreferences, Project, SortMode } from '@/types';

type SessionDialogState = { open: boolean; mode: 'general' | 'project'; path: string };
type ConfirmState = { kind: 'session' } | { kind: 'project'; project: Project } | { kind: 'backend'; profile: BackendProfile; instanceId: string } | null;

const SIDEBAR_MIN_WIDTH = 188;
const SIDEBAR_DEFAULT_WIDTH = 224;

function clampSidebarWidth(width: number) {
  const maxWidth = Math.max(SIDEBAR_MIN_WIDTH, Math.floor(window.innerWidth / 3));
  return Math.min(Math.max(width, SIDEBAR_MIN_WIDTH), maxWidth);
}

export function App() {
  const { preferences, persist } = usePreferences();
  const backends = useBackends();
  const backendReady = !desktopShell || !backends.loading;
  const workspaceSupported = !desktopShell || backends.active.kind === 'local' || backends.active.capabilities.includes('workspace-v1');
  const backendFeaturesReady = backendReady && workspaceSupported;
  const backendAddress = desktopShell ? (backendReady ? backends.active.address : '') : preferences.backendAddress;
  const backendKey = desktopShell ? (backendReady ? backends.active.id : 'desktop-loading') : backendAddress || 'embedded';
  const workspaceIcons = useWorkspaceIcons();
  const t = useMemo(() => (key: Parameters<typeof translate>[1]) => translate(preferences.locale, key), [preferences.locale]);
  const workspaces = useWorkspaces(backendAddress, backendKey, backendFeaturesReady);
  const sessionState = useSessions(backendAddress, backendKey, backendFeaturesReady);
  useDesktopTray(preferences, sessionState.sessions, sessionState.setActiveId);
  const [sidebarOpen, setSidebarOpen] = useState(() => window.innerWidth > 760);
  const [sidebarWidth, setSidebarWidth] = useState(() => {
    const saved = Number(localStorage.getItem('akironmux-sidebar-width')) || SIDEBAR_DEFAULT_WIDTH;
    return clampSidebarWidth(saved);
  });
  const [searchOpen, setSearchOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [sessionDialog, setSessionDialog] = useState<SessionDialogState>({ open: false, mode: 'general', path: '' });
  const [projectOpen, setProjectOpen] = useState(false);
  const [editingProject, setEditingProject] = useState<Project | null>(null);
  const [iconPath, setIconPath] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<ConfirmState>(null);
  const [imeComposing, setImeComposing] = useState(false);

  useEffect(() => {
    const clamp = () => setSidebarWidth(value => clampSidebarWidth(value));
    window.addEventListener('resize', clamp);
    return () => window.removeEventListener('resize', clamp);
  }, []);

  useEffect(() => {
    const start = () => setImeComposing(true);
    const end = () => setImeComposing(false);
    document.addEventListener('compositionstart', start);
    document.addEventListener('compositionend', end);
    return () => {
      document.removeEventListener('compositionstart', start);
      document.removeEventListener('compositionend', end);
    };
  }, []);

  useEffect(() => {
    if (!desktopShell || backends.loading || backends.active.kind !== 'remote' || backends.active.requiresAuth) return;
    const refresh = () => void backends.refreshActive(backends.active.id).catch(() => undefined);
    refresh();
    const timer = window.setInterval(refresh, 10_000);
    return () => window.clearInterval(timer);
  }, [backends.active.id, backends.active.kind, backends.active.requiresAuth, backends.loading, backends.refreshActive]);

  const openGeneralSession = () => setSessionDialog({ open: true, mode: 'general', path: workspaces.workspace.general_root });
  const openProjectSession = (path: string) => setSessionDialog({ open: true, mode: 'project', path });

  const saveProject = async (path: string, name: string, icon: WorkspaceIconName) => {
    const project = editingProject
      ? await sessionApi.updateProject(backendAddress, editingProject.id, { name, path })
      : await sessionApi.createProject(backendAddress, path, name);
    workspaceIcons.setIcon(`project:${project.id}`, icon);
    await workspaces.load();
  };

  const saveSettings = async (next: ClientPreferences, generalRoot: string) => {
    if (workspaceSupported) await sessionApi.updateSettings(backendAddress, { general_root: generalRoot });
    persist(next);
    if (workspaceSupported) await workspaces.load();
  };

  const updateSort = async (section: 'projects' | 'general' | 'other', mode: SortMode) => {
    await sessionApi.updateSort(backendAddress, section, mode);
    await workspaces.load();
  };

  const updateDirectorySort = async (path: string, mode: SortMode) => {
    await sessionApi.updateDirectorySort(backendAddress, path, mode);
    await workspaces.load();
  };

  const reorderItems = async (kind: 'projects' | 'directories' | 'sessions', scope: string, ids: string[]) => {
    await sessionApi.reorder(backendAddress, kind, scope, ids);
    await workspaces.load();
  };

  const handleSessionStatus = (session: (typeof sessionState.sessions)[number]) => {
    sessionState.update(session);
  };

  const handleSessionAttention = (session: (typeof sessionState.sessions)[number], kind: AttentionKind) => {
    if (session.id !== sessionState.activeId) sessionState.markAttention(session.id, kind);
    void notifySession(session, kind, preferences.locale);
  };

  const selectBackend = async (profile: BackendProfile) => {
    if (profile.kind === 'remote') {
      try {
        const health = await backends.test(profile);
        if (!profile.instanceId || profile.instanceId !== health.instanceId) {
          setConfirm({ kind: 'backend', profile, instanceId: health.instanceId });
          return;
        }
        if (profile.capabilities.join('\0') !== health.capabilities.join('\0')) {
          await backends.save(profile);
        }
      } catch {
        await backends.select(profile.id);
        return;
      }
    }
    await backends.select(profile.id);
  };

  const confirmTitle = confirm?.kind === 'project' ? t('remove') : confirm?.kind === 'backend' ? t('confirm') : t('closeTitle');
  const confirmBody = confirm?.kind === 'project' ? `${t('remove')}: ${confirm.project.name}` : confirm?.kind === 'backend' ? t('identityChanged') : t('closeBody');

  return (
    <TooltipProvider>
      <div className="app-background flex h-full flex-col">
        <header
          className="acrylic-shell relative z-20 flex h-9 shrink-0 items-center pl-2"
          data-tauri-drag-region={desktopShell ? '' : undefined}
          onDoubleClick={event => {
            if (desktopShell && !(event.target as HTMLElement).closest('button')) {
              void toggleDesktopMaximize();
            }
          }}
        >
          <Tooltip label={t('toggleSidebar')}>
            <Button variant="ghost" size="icon" onClick={() => setSidebarOpen(value => !value)}>
              <PanelLeft className="size-4" />
            </Button>
          </Tooltip>
          <strong
            className="pointer-events-none absolute left-1/2 -translate-x-1/2 truncate text-xs font-semibold"
            data-app-title
            data-tauri-drag-region={desktopShell ? '' : undefined}
          >
            AkironMux
          </strong>
          {desktopShell && backendReady && (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="ghost"
                  size="sm"
                  className="ml-1 max-w-44 gap-1.5 text-xs"
                  title={!workspaceSupported ? t('workspaceCapabilityUnavailable') : undefined}
                >
                  <Server className="size-4" />
                  <span className="truncate">{backends.active.kind === 'local' ? t('localBackend') : backends.active.name}</span>
                  <span className={`size-1.5 rounded-full ${workspaces.connected ? 'bg-emerald-500' : 'bg-destructive'}`} />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start" className="min-w-52">
                {backends.state.profiles.map(profile => (
                  <DropdownMenuItem key={profile.id} disabled={imeComposing} title={imeComposing ? t('finishComposition') : undefined} onSelect={() => void selectBackend(profile)}>
                    <Check className={profile.id === backends.active.id ? '' : 'opacity-0'} />
                    <span className="min-w-0 flex-1 truncate">{profile.kind === 'local' ? t('localBackend') : profile.name}</span>
                    <span className="text-[10px] text-muted-foreground">{profile.kind === 'local' ? t('localBackend') : t('remoteBackend')}</span>
                  </DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
          )}
          <span className="ml-auto hidden items-center gap-2 px-2 text-[11px] text-muted-foreground sm:flex" data-tauri-drag-region={desktopShell ? '' : undefined}>
            <span className={`size-1.5 rounded-full ${workspaces.connected ? 'bg-emerald-500' : 'bg-destructive'}`} />
            {t(workspaces.connected ? 'connected' : 'disconnected')}
          </span>
          <WindowControls t={t} />
        </header>
        <div className="relative flex min-h-0 flex-1">
          <button
            className="mobile-scrim fixed inset-0 top-9 z-20 hidden bg-black/30"
            data-open={sidebarOpen}
            aria-label={t('toggleSidebar')}
            onClick={() => setSidebarOpen(false)}
          />
          <AppSidebar
            key={backendKey}
            backendKey={backendKey}
            workspace={workspaces.workspace}
            settings={workspaces.settings}
            icons={workspaceIcons.icons}
            activeNativeId={sessionState.active?.native_session_id}
            attentionByNativeId={sessionState.nativeAttention}
            open={sidebarOpen}
            workspaceEnabled={workspaceSupported}
            width={sidebarWidth}
            locale={preferences.locale}
            t={t}
            onSearch={() => setSearchOpen(true)}
            onRefresh={() => void workspaces.refresh()}
            onNewGeneral={openGeneralSession}
            onNewProjectSession={openProjectSession}
            onAddProject={() => {
              setEditingProject(null);
              setProjectOpen(true);
            }}
            onEditProject={project => {
              setEditingProject(project);
              setProjectOpen(true);
            }}
            onEditDirectoryIcon={path => setIconPath(path)}
            onToggleProjectPin={project => void sessionApi.updateProject(backendAddress, project.id, { pinned: !project.pinned }).then(workspaces.load)}
            onDeleteProject={project => setConfirm({ kind: 'project', project })}
            onResume={item => void sessionState.resume(item)}
            onSort={(section, mode) => void updateSort(section, mode)}
            onDirectorySort={(path, mode) => void updateDirectorySort(path, mode)}
            onReorder={(kind, scope, ids) => void reorderItems(kind, scope, ids)}
            onSettings={() => setSettingsOpen(true)}
            onWidthChange={width => {
              const next = clampSidebarWidth(width);
              setSidebarWidth(next);
              localStorage.setItem('akironmux-sidebar-width', String(next));
            }}
          />
          <WorkspaceShell
            backendAddress={backendAddress}
            sessions={sessionState.sessions}
            active={sessionState.active}
            activeId={sessionState.activeId}
            attention={sessionState.attention}
            terminalFontSize={preferences.terminalFontSize}
            detailsOpen={detailsOpen}
            connected={workspaces.connected}
            workspaceEnabled={workspaceSupported}
            locale={preferences.locale}
            t={t}
            onSelect={sessionState.setActiveId}
            onStatus={handleSessionStatus}
            onAttention={handleSessionAttention}
            onNew={openGeneralSession}
            onDetails={() => setDetailsOpen(value => !value)}
            onRestart={() => sessionState.active && void sessionApi.restartSession(backendAddress, sessionState.active.id)}
            onClose={() => setConfirm({ kind: 'session' })}
          />
        </div>
      </div>

      <SessionDialog
        open={sessionDialog.open}
        mode={sessionDialog.mode}
        backendAddress={backendAddress}
        initialDirectory={sessionDialog.path}
        t={t}
        onOpenChange={open => setSessionDialog(value => ({ ...value, open }))}
        onCreate={sessionState.create}
      />
      <ProjectDialog
        open={projectOpen}
        project={editingProject}
        icon={workspaceIcons.icons[`project:${editingProject?.id || ''}`] || 'folder'}
        backendAddress={backendAddress}
        initialPath=""
        t={t}
        onOpenChange={setProjectOpen}
        onSave={saveProject}
      />
      <IconDialog
        open={iconPath !== null}
        title={`${t('editIcon')}${iconPath ? ` · ${basename(iconPath)}` : ''}`}
        value={(iconPath && workspaceIcons.icons[`directory:${iconPath}`]) || 'folder'}
        t={t}
        onOpenChange={open => {
          if (!open) setIconPath(null);
        }}
        onSave={icon => {
          if (iconPath) workspaceIcons.setIcon(`directory:${iconPath}`, icon);
        }}
      />
      <SearchDialog open={searchOpen} workspace={workspaces.workspace} t={t} onOpenChange={setSearchOpen} onResume={item => void sessionState.resume(item)} />
      <SettingsDialog
        open={settingsOpen}
        preferences={preferences}
        backends={backends}
        workspaceEnabled={workspaceSupported}
        generalRoot={workspaces.settings.general_root || workspaces.workspace.general_root}
        t={t}
        onOpenChange={setSettingsOpen}
        onSave={saveSettings}
      />
      <ConfirmDialog
        open={confirm !== null}
        title={confirmTitle}
        body={confirmBody}
        confirmLabel={confirm?.kind === 'project' ? t('remove') : confirm?.kind === 'backend' ? t('confirm') : t('close')}
        cancelLabel={t('cancel')}
        destructive
        onOpenChange={open => {
          if (!open) setConfirm(null);
        }}
        onConfirm={() => {
          if (confirm?.kind === 'session' && sessionState.active) void sessionState.close(sessionState.active.id);
          if (confirm?.kind === 'project') void sessionApi.deleteProject(backendAddress, confirm.project.id).then(workspaces.load);
          if (confirm?.kind === 'backend') {
            void backends.save(confirm.profile, confirm.instanceId).then(() => backends.select(confirm.profile.id));
          }
        }}
      />
    </TooltipProvider>
  );
}
