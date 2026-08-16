import * as Collapsible from '@radix-ui/react-collapsible';
import { Bell, Check, ChevronRight, CircleStop, Ellipsis, FolderPlus, Image, Pencil, Pin, Plus, RefreshCw, Search, Settings, Trash2 } from 'lucide-react';
import { useRef } from 'react';
import { AgentIcon } from '@/shared/components/agent-icon';
import { WorkspaceIcon, type WorkspaceIconName } from '@/shared/components/workspace-icon';
import { basename, cn } from '@/shared/lib/utils';
import { Button } from '@/shared/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from '@/shared/ui/dropdown-menu';
import { Tooltip } from '@/shared/ui/tooltip';
import type { AttentionKind, HistoryItem, Locale, Project, SettingsResponse, SortMode, WorkspaceResponse } from '@/types';
import type { MessageKey } from '@/shared/lib/i18n';

interface AppSidebarProps {
  workspace: WorkspaceResponse;
  settings: SettingsResponse;
  icons: Record<string, WorkspaceIconName>;
  activeNativeId?: string | null;
  attentionByNativeId: Record<string, AttentionKind>;
  open: boolean;
  width: number;
  locale: Locale;
  t: (key: MessageKey) => string;
  onSearch: () => void;
  onRefresh: () => void;
  onNewGeneral: () => void;
  onNewProjectSession: (path: string) => void;
  onAddProject: () => void;
  onEditProject: (project: Project) => void;
  onEditDirectoryIcon: (path: string) => void;
  onToggleProjectPin: (project: Project) => void;
  onDeleteProject: (project: Project) => void;
  onResume: (item: HistoryItem) => void;
  onSort: (section: 'projects' | 'general' | 'other', mode: SortMode) => void;
  onDirectorySort: (path: string, mode: SortMode) => void;
  onReorder: (kind: 'projects' | 'directories' | 'sessions', scope: string, ids: string[]) => void;
  onSettings: () => void;
  onWidthChange: (width: number) => void;
}

function SortMenu({ section, value, t, onSort }: { section: 'projects' | 'general' | 'other'; value: SortMode; t: AppSidebarProps['t']; onSort: AppSidebarProps['onSort'] }) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button data-section-sort={section} variant="ghost" size="icon-sm" aria-label={t('sort')}>
          <Ellipsis />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" side="bottom" sideOffset={4} className="min-w-36 text-xs">
        {(['priority', 'recent', 'manual'] as const).map(mode => (
          <DropdownMenuItem className="text-xs" key={mode} onSelect={() => onSort(section, mode)}>
            <Check className={cn(value !== mode && 'opacity-0')} />
            {t(mode)}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function moveId(ids: string[], moved: string, target: string) {
  const next = ids.filter(id => id !== moved);
  const targetIndex = next.indexOf(target);
  next.splice(targetIndex < 0 ? next.length : targetIndex, 0, moved);
  return next;
}

function setDragData(event: React.DragEvent, kind: string, scope: string, id: string) {
  event.dataTransfer.effectAllowed = 'move';
  event.dataTransfer.setData('application/x-akmux-order', JSON.stringify({ kind, scope, id }));
}

function getDragData(event: React.DragEvent) {
  try {
    return JSON.parse(event.dataTransfer.getData('application/x-akmux-order')) as { kind: string; scope: string; id: string };
  } catch {
    return null;
  }
}

function HistoryRow({ item, active, attention, draggable, onDragStart, onDrop, onResume }: { item: HistoryItem; active: boolean; attention?: AttentionKind; draggable: boolean; onDragStart: (event: React.DragEvent) => void; onDrop: (event: React.DragEvent) => void; onResume: (item: HistoryItem) => void }) {
  return (
    <button
      className="history-row"
      data-active={active}
      draggable={draggable}
      onDragStart={onDragStart}
      onDragOver={event => {
        if (draggable) event.preventDefault();
      }}
      onDrop={onDrop}
      onClick={() => onResume(item)}
      title={item.cwd}
    >
      <AgentIcon agent={item.agent} className="size-6" />
      <span className="min-w-0 flex-1">
        <strong className="block truncate text-xs font-medium">{item.title}</strong>
        <small className="block truncate text-[11px] text-foreground/75">
          {item.agent === 'codex' ? 'Codex' : 'Claude Code'} · {basename(item.cwd)}
        </small>
      </span>
      {attention === 'input' ? <Bell className="session-signal session-signal-input" /> : attention === 'exited' ? <CircleStop className="session-signal session-signal-exited" /> : active && <span className="size-1.5 rounded-full bg-emerald-500" />}
    </button>
  );
}

function HistoryList({
  items,
  activeNativeId,
  attentionByNativeId,
  sortMode,
  scope,
  onReorder,
  onResume,
  empty,
}: {
  items: HistoryItem[];
  activeNativeId?: string | null;
  attentionByNativeId: Record<string, AttentionKind>;
  sortMode: SortMode;
  scope: string;
  onReorder: AppSidebarProps['onReorder'];
  onResume: (item: HistoryItem) => void;
  empty: string;
}) {
  if (!items.length) return <div className="px-9 py-2 text-xs text-muted-foreground">{empty}</div>;
  return (
    <div className="space-y-0.5 pb-1 pl-4">
      {items.map(item => {
        const itemId = `${item.agent}:${item.id}`;
        return (
        <HistoryRow
          key={itemId}
          item={item}
          active={item.id === activeNativeId}
          attention={attentionByNativeId[itemId]}
          draggable={sortMode === 'manual'}
          onDragStart={event => setDragData(event, 'sessions', scope, itemId)}
          onDrop={event => {
            const payload = getDragData(event);
            if (sortMode !== 'manual' || payload?.kind !== 'sessions' || payload.scope !== scope) return;
            event.preventDefault();
            onReorder('sessions', scope, moveId(items.map(entry => `${entry.agent}:${entry.id}`), payload.id, itemId));
          }}
          onResume={onResume}
        />
        );
      })}
    </div>
  );
}

function SectionHeading({ title, children }: { title: string; children?: React.ReactNode }) {
  return (
    <div className="flex h-8 items-center px-2 text-xs font-medium text-muted-foreground">
      <Collapsible.Trigger className="flex min-w-0 flex-1 items-center gap-1.5 text-left">
        <ChevronRight className="size-3.5 shrink-0 transition-transform group-data-[state=open]/section:rotate-90" />
        <span className="truncate">{title}</span>
      </Collapsible.Trigger>
      <span className="ml-auto flex items-center gap-0.5">{children}</span>
    </div>
  );
}

export function AppSidebar(props: AppSidebarProps) {
  const { workspace, settings, t } = props;
  const resizeStart = useRef<{ pointerX: number; width: number } | null>(null);

  const startResize = (event: React.PointerEvent<HTMLDivElement>) => {
    if (window.innerWidth <= 760) return;
    resizeStart.current = { pointerX: event.clientX, width: props.width };
    event.currentTarget.setPointerCapture(event.pointerId);
    document.documentElement.dataset.sidebarResizing = 'true';
  };

  const resize = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!resizeStart.current) return;
    const next = resizeStart.current.width + event.clientX - resizeStart.current.pointerX;
    props.onWidthChange(Math.min(next, window.innerWidth / 3));
  };

  const stopResize = (event: React.PointerEvent<HTMLDivElement>) => {
    resizeStart.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    delete document.documentElement.dataset.sidebarResizing;
  };

  const resizeWithKeyboard = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
    event.preventDefault();
    props.onWidthChange(props.width + (event.key === 'ArrowRight' ? 12 : -12));
  };

  return (
    <aside
      className="desktop-sidebar acrylic-shell relative flex min-h-0 shrink-0 flex-col transition-[width,transform,opacity] duration-200"
      data-open={props.open}
      style={{ width: props.width }}
    >
      <div className="flex gap-1.5 px-3 pb-2 pt-3">
        <Button id="new-general" variant="ghost" className="h-9 flex-1 justify-start px-2.5 text-foreground" onClick={props.onNewGeneral}>
          <Plus />
          <span className="sidebar-new-label">{t('newSession')}</span>
        </Button>
        <Tooltip label={t('search')}>
          <Button id="search-button" variant="ghost" size="icon" onClick={props.onSearch}>
            <Search />
          </Button>
        </Tooltip>
        <Tooltip label={t('refresh')}>
          <Button variant="ghost" size="icon" onClick={props.onRefresh}>
            <RefreshCw />
          </Button>
        </Tooltip>
      </div>

      <div data-scroll-region="sidebar" className="min-h-0 flex-1 overflow-y-auto px-2 pb-4">
        <Collapsible.Root className="group/section mb-2">
          <SectionHeading title={t('projects')}>
            <SortMenu section="projects" value={settings.project_sort} t={t} onSort={props.onSort} />
            <Tooltip label={t('addProject')}>
              <Button variant="ghost" size="icon-sm" onClick={props.onAddProject}>
                <Plus />
              </Button>
            </Tooltip>
          </SectionHeading>
          <Collapsible.Content>
            {workspace.projects.length ? (
              workspace.projects.map(group => (
                <Collapsible.Root key={group.project.id} defaultOpen={false} className="group/project">
                  <div
                    className="flex min-h-9 items-center rounded-lg px-1 hover:bg-accent"
                    draggable
                    onDragStart={event => setDragData(event, 'projects', 'projects', group.project.id)}
                    onDragOver={event => event.preventDefault()}
                    onDrop={event => {
                      const payload = getDragData(event);
                      if (payload?.kind !== 'projects') return;
                      event.preventDefault();
                      props.onReorder('projects', 'projects', moveId(workspace.projects.map(item => item.project.id), payload.id, group.project.id));
                    }}
                  >
                    <Collapsible.Trigger className="grid size-7 shrink-0 place-items-center text-muted-foreground">
                      <ChevronRight className="size-4 transition-transform group-data-[state=open]/project:rotate-90" />
                    </Collapsible.Trigger>
                    <WorkspaceIcon name={props.icons[`project:${group.project.id}`]} className="mr-2 text-foreground/80" />
                    <button className="min-w-0 flex-1 truncate text-left text-xs font-medium" title={group.project.path}>
                      {group.project.name}
                    </button>
                    {group.project.pinned && <Pin className="mr-1 size-3 text-primary" />}
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button data-project-menu={group.project.id} variant="ghost" size="icon-sm" aria-label={t('menu')}>
                          <Ellipsis />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent id="context-menu" align="start">
                        <DropdownMenuItem onSelect={() => props.onNewProjectSession(group.project.path)}>
                          <FolderPlus />
                          {t('newSession')}
                        </DropdownMenuItem>
                        <DropdownMenuItem onSelect={() => props.onEditProject(group.project)}>
                          <Pencil />
                          {t('editProject')}
                        </DropdownMenuItem>
                        <DropdownMenuItem onSelect={() => props.onToggleProjectPin(group.project)}>
                          <Pin />
                          {t(group.project.pinned ? 'unpin' : 'pin')}
                        </DropdownMenuItem>
                        <DropdownMenuSeparator />
                        <DropdownMenuItem className="text-destructive focus:bg-destructive/10" onSelect={() => props.onDeleteProject(group.project)}>
                          <Trash2 />
                          {t('remove')}
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                    <Button variant="ghost" size="icon-sm" onClick={() => props.onNewProjectSession(group.project.path)} aria-label={t('newSession')}>
                      <Plus />
                    </Button>
                  </div>
                  <Collapsible.Content>
                    <HistoryList
                      items={group.history}
                      activeNativeId={props.activeNativeId}
                      attentionByNativeId={props.attentionByNativeId}
                      sortMode={settings.project_sort}
                      scope={`project:${group.project.id}`}
                      onReorder={props.onReorder}
                      onResume={props.onResume}
                      empty={t('noHistory')}
                    />
                  </Collapsible.Content>
                </Collapsible.Root>
              ))
            ) : (
              <div className="px-3 py-2 text-xs text-muted-foreground">{t('noHistory')}</div>
            )}
          </Collapsible.Content>
        </Collapsible.Root>

        <Collapsible.Root className="group/section mb-2">
          <SectionHeading title={t('general')}>
            <SortMenu section="general" value={settings.general_sort} t={t} onSort={props.onSort} />
            <Tooltip label={t('newGeneral')}>
              <Button variant="ghost" size="icon-sm" onClick={props.onNewGeneral}>
                <Plus />
              </Button>
            </Tooltip>
          </SectionHeading>
          <Collapsible.Content>
            {workspace.general.length ? (
              workspace.general.map(group => (
                <Collapsible.Root
                  key={group.path}
                  defaultOpen={false}
                  className="group/directory opacity-100 data-[unavailable=true]:opacity-55"
                  data-unavailable={!group.available}
                >
                  <div
                    className="flex min-h-9 items-center rounded-lg px-1 hover:bg-accent"
                    draggable
                    onDragStart={event => setDragData(event, 'directories', 'general', group.path)}
                    onDragOver={event => event.preventDefault()}
                    onDrop={event => {
                      const payload = getDragData(event);
                      if (payload?.kind !== 'directories' || payload.scope !== 'general') return;
                      event.preventDefault();
                      props.onReorder('directories', 'general', moveId(workspace.general.map(item => item.path), payload.id, group.path));
                    }}
                  >
                    <Collapsible.Trigger className="flex min-w-0 flex-1 items-center text-left">
                      <ChevronRight className="mx-1.5 size-4 shrink-0 text-muted-foreground transition-transform group-data-[state=open]/directory:rotate-90" />
                      <WorkspaceIcon name={props.icons[`directory:${group.path}`]} className="mr-2 text-foreground/80" />
                      <span className="min-w-0 flex-1 truncate text-xs" title={group.path}>
                        {basename(group.path)}
                      </span>
                      <small className="px-1.5 text-[10px] text-foreground/80">{group.available ? group.items.length : t('unavailable')}</small>
                    </Collapsible.Trigger>
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button data-directory-menu={group.path} variant="ghost" size="icon-sm" aria-label={t('menu')}>
                          <Ellipsis />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="start" className="min-w-36">
                        {(['priority', 'recent', 'manual'] as const).map(mode => (
                          <DropdownMenuItem className="text-xs" key={mode} onSelect={() => props.onDirectorySort(group.path, mode)}>
                            <Check className={cn((settings.directory_sort[group.path] || settings.general_sort) !== mode && 'opacity-0')} />
                            {t(mode)}
                          </DropdownMenuItem>
                        ))}
                        <DropdownMenuSeparator />
                        <DropdownMenuItem onSelect={() => props.onEditDirectoryIcon(group.path)}>
                          <Image />
                          {t('editIcon')}
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </div>
                  <Collapsible.Content>
                    <HistoryList
                      items={group.items}
                      activeNativeId={props.activeNativeId}
                      attentionByNativeId={props.attentionByNativeId}
                      sortMode={settings.directory_sort[group.path] || settings.general_sort}
                      scope={`directory:${group.path}`}
                      onReorder={props.onReorder}
                      onResume={props.onResume}
                      empty={t('noHistory')}
                    />
                  </Collapsible.Content>
                </Collapsible.Root>
              ))
            ) : (
              <div className="px-3 py-2 text-xs text-muted-foreground">{t('noHistory')}</div>
            )}
          </Collapsible.Content>
        </Collapsible.Root>

        <Collapsible.Root className="group/section">
          <SectionHeading title={t('other')}>
            <SortMenu section="other" value={settings.other_sort} t={t} onSort={props.onSort} />
          </SectionHeading>
          <Collapsible.Content>
            {workspace.other.length ? (
              workspace.other.map(group => (
                <Collapsible.Root key={group.path} defaultOpen={false} className="group/directory">
                  <div
                    className="flex min-h-9 items-center rounded-lg px-1 hover:bg-accent"
                    draggable
                    onDragStart={event => setDragData(event, 'directories', 'other', group.path)}
                    onDragOver={event => event.preventDefault()}
                    onDrop={event => {
                      const payload = getDragData(event);
                      if (payload?.kind !== 'directories' || payload.scope !== 'other') return;
                      event.preventDefault();
                      props.onReorder('directories', 'other', moveId(workspace.other.map(item => item.path), payload.id, group.path));
                    }}
                  >
                    <Collapsible.Trigger className="flex min-w-0 flex-1 items-center text-left">
                      <ChevronRight className="mx-1.5 size-4 shrink-0 text-muted-foreground transition-transform group-data-[state=open]/directory:rotate-90" />
                      <WorkspaceIcon name={props.icons[`directory:${group.path}`]} className="mr-2 text-foreground/75" />
                      <span className="min-w-0 flex-1 truncate text-xs" title={group.path}>
                        {basename(group.path)}
                      </span>
                      <small className="px-1.5 text-[10px] text-foreground/80">{group.items.length}</small>
                    </Collapsible.Trigger>
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button data-directory-menu={group.path} variant="ghost" size="icon-sm" aria-label={t('menu')}>
                          <Ellipsis />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="start" className="min-w-36">
                        {(['priority', 'recent', 'manual'] as const).map(mode => (
                          <DropdownMenuItem className="text-xs" key={mode} onSelect={() => props.onDirectorySort(group.path, mode)}>
                            <Check className={cn((settings.directory_sort[group.path] || settings.other_sort) !== mode && 'opacity-0')} />
                            {t(mode)}
                          </DropdownMenuItem>
                        ))}
                        <DropdownMenuSeparator />
                        <DropdownMenuItem onSelect={() => props.onEditDirectoryIcon(group.path)}>
                          <Image />
                          {t('editIcon')}
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </div>
                  <Collapsible.Content>
                    <HistoryList
                      items={group.items}
                      activeNativeId={props.activeNativeId}
                      attentionByNativeId={props.attentionByNativeId}
                      sortMode={settings.directory_sort[group.path] || settings.other_sort}
                      scope={`directory:${group.path}`}
                      onReorder={props.onReorder}
                      onResume={props.onResume}
                      empty={t('noHistory')}
                    />
                  </Collapsible.Content>
                </Collapsible.Root>
              ))
            ) : (
              <div className="px-3 py-2 text-xs text-muted-foreground">{t('noHistory')}</div>
            )}
          </Collapsible.Content>
        </Collapsible.Root>
      </div>

      <div className="border-t border-border/60 p-2.5">
        <Button variant="ghost" className="h-10 w-full justify-start px-2.5 text-foreground" onClick={props.onSettings}>
          <Settings />
          {t('settings')}
        </Button>
      </div>
      <div
        className="sidebar-resize-handle"
        data-sidebar-resize-handle
        role="separator"
        tabIndex={0}
        aria-orientation="vertical"
        aria-label="Resize sidebar"
        aria-valuemin={188}
        aria-valuemax={Math.floor(window.innerWidth / 3)}
        aria-valuenow={Math.round(props.width)}
        onPointerDown={startResize}
        onPointerMove={resize}
        onPointerUp={stopResize}
        onPointerCancel={stopResize}
        onKeyDown={resizeWithKeyboard}
      />
    </aside>
  );
}
