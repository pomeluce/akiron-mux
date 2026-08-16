import * as Collapsible from '@radix-ui/react-collapsible';
import { Bell, Check, ChevronRight, CircleStop, Ellipsis, FolderPlus, Image, Pencil, Pin, Plus, RefreshCw, Search, Settings, Trash2 } from 'lucide-react';
import { useEffect, useRef } from 'react';
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

type ReorderKind = 'projects' | 'directories' | 'sessions';

interface ReorderPayload {
  kind: ReorderKind;
  scope: string;
  id: string;
  ids: string[];
}

interface PointerReorder extends ReorderPayload {
  pointerId: number;
  startX: number;
  startY: number;
  active: boolean;
  source: HTMLElement;
  target: HTMLElement | null;
}

function usePointerReorder(onReorder: AppSidebarProps['onReorder']) {
  const activeRef = useRef<PointerReorder | null>(null);
  const onReorderRef = useRef(onReorder);
  const suppressClickRef = useRef(false);
  onReorderRef.current = onReorder;

  useEffect(() => {
    const clearTarget = (drag: PointerReorder) => {
      drag.target?.removeAttribute('data-reorder-target');
      drag.target = null;
    };
    const cleanup = (drag: PointerReorder) => {
      clearTarget(drag);
      drag.source.removeAttribute('data-reordering');
      delete document.documentElement.dataset.sidebarReordering;
      activeRef.current = null;
    };
    const move = (event: PointerEvent) => {
      const drag = activeRef.current;
      if (!drag || drag.pointerId !== event.pointerId) return;
      if (!drag.active) {
        const distance = Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY);
        if (distance < 6) return;
        drag.active = true;
        drag.source.dataset.reordering = 'true';
        document.documentElement.dataset.sidebarReordering = 'true';
      }
      event.preventDefault();
      const candidate = document.elementFromPoint(event.clientX, event.clientY)?.closest<HTMLElement>('[data-reorder-id]') || null;
      const target = candidate?.dataset.reorderKind === drag.kind && candidate.dataset.reorderScope === drag.scope ? candidate : null;
      if (target === drag.target) return;
      clearTarget(drag);
      drag.target = target;
      target?.setAttribute('data-reorder-target', 'true');
    };
    const finish = (event: PointerEvent) => {
      const drag = activeRef.current;
      if (!drag || drag.pointerId !== event.pointerId) return;
      const targetId = drag.target?.dataset.reorderId;
      const active = drag.active;
      cleanup(drag);
      if (!active) return;
      event.preventDefault();
      suppressClickRef.current = true;
      window.setTimeout(() => {
        suppressClickRef.current = false;
      }, 0);
      if (targetId && targetId !== drag.id) onReorderRef.current(drag.kind, drag.scope, moveId(drag.ids, drag.id, targetId));
    };
    const cancel = (event: PointerEvent) => {
      const drag = activeRef.current;
      if (drag?.pointerId === event.pointerId) cleanup(drag);
    };
    const suppressClick = (event: MouseEvent) => {
      if (!suppressClickRef.current) return;
      suppressClickRef.current = false;
      event.preventDefault();
      event.stopImmediatePropagation();
    };

    document.addEventListener('pointermove', move, { passive: false });
    document.addEventListener('pointerup', finish, true);
    document.addEventListener('pointercancel', cancel, true);
    document.addEventListener('click', suppressClick, true);
    return () => {
      if (activeRef.current) cleanup(activeRef.current);
      document.removeEventListener('pointermove', move);
      document.removeEventListener('pointerup', finish, true);
      document.removeEventListener('pointercancel', cancel, true);
      document.removeEventListener('click', suppressClick, true);
    };
  }, []);

  return (event: React.PointerEvent<HTMLElement>, payload: ReorderPayload) => {
    if (event.button !== 0 || activeRef.current) return;
    activeRef.current = {
      ...payload,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      active: false,
      source: event.currentTarget,
      target: null,
    };
  };
}

function HistoryRow({ item, active, attention, reorder, onPointerReorder, onResume }: { item: HistoryItem; active: boolean; attention?: AttentionKind; reorder: ReorderPayload | null; onPointerReorder: (event: React.PointerEvent<HTMLElement>, payload: ReorderPayload) => void; onResume: (item: HistoryItem) => void }) {
  return (
    <button
      className="history-row"
      data-active={active}
      data-reorder-enabled={Boolean(reorder)}
      data-reorder-kind={reorder?.kind}
      data-reorder-scope={reorder?.scope}
      data-reorder-id={reorder?.id}
      onPointerDown={reorder ? event => onPointerReorder(event, reorder) : undefined}
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
  onPointerReorder,
  onResume,
  empty,
}: {
  items: HistoryItem[];
  activeNativeId?: string | null;
  attentionByNativeId: Record<string, AttentionKind>;
  sortMode: SortMode;
  scope: string;
  onPointerReorder: (event: React.PointerEvent<HTMLElement>, payload: ReorderPayload) => void;
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
          reorder={sortMode === 'manual' ? { kind: 'sessions', scope, id: itemId, ids: items.map(entry => `${entry.agent}:${entry.id}`) } : null}
          onPointerReorder={onPointerReorder}
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
  const startReorder = usePointerReorder(props.onReorder);

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
                <Collapsible.Root key={group.project.id} defaultOpen={false} className="group/project" data-project-group={group.project.id}>
                  <div
                    className="draggable-row flex min-h-9 items-center rounded-lg px-1 hover:bg-accent"
                    data-project-row={group.project.id}
                    data-reorder-enabled="true"
                    data-reorder-kind="projects"
                    data-reorder-scope="projects"
                    data-reorder-id={group.project.id}
                    onPointerDown={event =>
                      startReorder(event, { kind: 'projects', scope: 'projects', id: group.project.id, ids: workspace.projects.map(item => item.project.id) })
                    }
                  >
                    <Collapsible.Trigger className="grid size-7 shrink-0 place-items-center text-muted-foreground">
                      <ChevronRight className="size-4 transition-transform group-data-[state=open]/project:rotate-90" />
                    </Collapsible.Trigger>
                    <WorkspaceIcon name={props.icons[`project:${group.project.id}`]} className="mr-2 text-foreground/80" />
                    <Collapsible.Trigger className="min-w-0 flex-1 truncate text-left text-xs font-medium" title={group.project.path}>
                      {group.project.name}
                    </Collapsible.Trigger>
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
                      onPointerReorder={startReorder}
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
                    className="draggable-row flex min-h-9 items-center rounded-lg px-1 hover:bg-accent"
                    data-directory-row={group.path}
                    data-reorder-enabled="true"
                    data-reorder-kind="directories"
                    data-reorder-scope="general"
                    data-reorder-id={group.path}
                    onPointerDown={event => startReorder(event, { kind: 'directories', scope: 'general', id: group.path, ids: workspace.general.map(item => item.path) })}
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
                      onPointerReorder={startReorder}
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
                    className="draggable-row flex min-h-9 items-center rounded-lg px-1 hover:bg-accent"
                    data-directory-row={group.path}
                    data-reorder-enabled="true"
                    data-reorder-kind="directories"
                    data-reorder-scope="other"
                    data-reorder-id={group.path}
                    onPointerDown={event => startReorder(event, { kind: 'directories', scope: 'other', id: group.path, ids: workspace.other.map(item => item.path) })}
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
                      onPointerReorder={startReorder}
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
