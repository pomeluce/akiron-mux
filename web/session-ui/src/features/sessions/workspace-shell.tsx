import { CircleAlert, PanelRight, Plus, RefreshCw, SquareTerminal, X } from 'lucide-react';
import { AgentIcon } from '@/shared/components/agent-icon';
import { basename, cn } from '@/shared/lib/utils';
import { Button } from '@/shared/ui/button';
import { Tooltip } from '@/shared/ui/tooltip';
import type { Locale, SessionInfo } from '@/types';
import type { MessageKey } from '@/shared/lib/i18n';
import { TerminalView } from './terminal-view';

interface WorkspaceShellProps {
  backendAddress: string;
  sessions: SessionInfo[];
  active?: SessionInfo;
  activeId: string | null;
  detailsOpen: boolean;
  connected: boolean;
  locale: Locale;
  t: (key: MessageKey) => string;
  onSelect: (id: string) => void;
  onStatus: (session: SessionInfo) => void;
  onNew: () => void;
  onDetails: () => void;
  onRestart: () => void;
  onClose: () => void;
}

export function WorkspaceShell(props: WorkspaceShellProps) {
  const { active, sessions, t } = props;
  return (
    <main className="workspace-surface ml-1.5 mt-1.5 flex min-w-0 flex-1 flex-col overflow-hidden rounded-tl-xl border border-border/60 bg-surface text-foreground max-[760px]:ml-1 max-[760px]:mt-1 max-[760px]:rounded-tl-lg">
      {sessions.length > 0 && (
        <div className="flex h-11 shrink-0 items-center gap-1 border-b border-border px-2">
          <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
            {sessions.map(session => (
              <button
                key={session.id}
                className={cn(
                  'flex h-8 min-w-32 max-w-56 shrink-0 items-center gap-1.5 rounded-lg px-2.5 text-xs text-muted-foreground hover:bg-accent',
                  session.id === props.activeId && 'bg-surface-raised text-foreground',
                )}
                onClick={() => props.onSelect(session.id)}
              >
                <AgentIcon agent={session.agent} className="size-5 rounded" />
                <span className="min-w-0 flex-1 truncate text-left">{session.title}</span>
                <span
                  className={cn(
                    'size-1.5 rounded-full bg-muted-foreground',
                    session.status === 'running' && 'bg-emerald-500',
                    (session.status === 'error' || session.status === 'exited') && 'bg-destructive',
                  )}
                />
              </button>
            ))}
          </div>
          {active && (
            <div className="stage-actions flex shrink-0 items-center gap-0.5 border-l border-border pl-1.5">
              <Tooltip label={t('details')}>
                <Button variant="ghost" size="icon" onClick={props.onDetails}>
                  <PanelRight />
                </Button>
              </Tooltip>
              <Tooltip label={t('restart')}>
                <Button variant="ghost" size="icon" disabled={active.status === 'starting'} onClick={props.onRestart}>
                  <RefreshCw />
                </Button>
              </Tooltip>
              <Tooltip label={t('close')}>
                <Button variant="destructive" size="icon" onClick={props.onClose}>
                  <X />
                </Button>
              </Tooltip>
            </div>
          )}
        </div>
      )}

      {active ? (
        <>
          <div className="relative min-h-0 flex-1 overflow-hidden bg-[#0b0f12]">
            <div className="terminal-stack terminal-surface">
              {sessions.map(session => (
                <TerminalView key={session.id} backendAddress={props.backendAddress} session={session} active={session.id === props.activeId} onStatus={props.onStatus} />
              ))}
            </div>
            <aside
              className={cn(
                'absolute inset-y-0 right-0 z-10 w-[min(340px,78%)] translate-x-full border-l border-white/10 bg-[#15212c]/96 p-5 text-slate-100 shadow-2xl backdrop-blur-xl transition-transform',
                props.detailsOpen && 'translate-x-0',
              )}
            >
              <div className="mb-5 flex items-center">
                <strong className="text-sm">{t('details')}</strong>
                <Button className="ml-auto text-slate-300 hover:bg-white/10 hover:text-white" variant="ghost" size="icon-sm" onClick={props.onDetails}>
                  <X />
                </Button>
              </div>
              <dl className="space-y-4 text-xs">
                <Detail label={t('agent')} value={active.agent === 'codex' ? 'Codex' : 'Claude Code'} />
                <Detail label={t('status')} value={t(active.status)} />
                <Detail label={t('directory')} value={active.cwd} />
                <Detail label={t('created')} value={new Date(active.created_at_ms).toLocaleString(props.locale)} />
              </dl>
              {active.error && (
                <div className="mt-5 flex gap-2 rounded-lg bg-red-500/12 p-3 text-xs text-red-200">
                  <CircleAlert className="size-4 shrink-0" />
                  {active.error}
                </div>
              )}
            </aside>
          </div>
        </>
      ) : (
        <div className="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden">
          <div className="max-w-md px-8 text-center">
            <div className="mx-auto mb-5 grid size-12 place-items-center rounded-xl bg-primary/10 text-primary">
              <SquareTerminal className="size-6" />
            </div>
            <h1 className="m-0 text-xl font-semibold">{t('emptyTitle')}</h1>
            <p className="mx-auto mt-2 max-w-none whitespace-nowrap text-sm text-muted-foreground max-[520px]:whitespace-normal">{t('emptyBody')}</p>
            <Button variant="secondary" className="mt-5" onClick={props.onNew}>
              <Plus />
              {t('newSession')}
            </Button>
          </div>
        </div>
      )}

      <footer className="relative z-20 flex h-7 shrink-0 items-center gap-2 border-t border-border bg-surface px-3 text-[10px] text-muted-foreground">
        <span className={cn('size-1.5 rounded-full', props.connected ? 'bg-emerald-500' : 'bg-destructive')} />
        <span>{t(props.connected ? 'connected' : 'disconnected')}</span>
        {active && <span className="min-w-0 truncate">· {basename(active.cwd)}</span>}
        <span className="ml-auto">AkironMux</span>
      </footer>
    </main>
  );
}

function Detail({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <dt className="mb-1 text-slate-400">{label}</dt>
      <dd className="m-0 break-words text-slate-100">{value}</dd>
    </div>
  );
}
