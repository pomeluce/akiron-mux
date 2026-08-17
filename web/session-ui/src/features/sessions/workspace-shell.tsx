import { Bell, CircleAlert, CircleCheck, Info, Plus, RefreshCw, SquareTerminal, X } from 'lucide-react';
import { useEffect, useState } from 'react';
import { AgentIcon } from '@/shared/components/agent-icon';
import { sessionApi } from '@/shared/lib/api';
import { basename, cn } from '@/shared/lib/utils';
import { Button } from '@/shared/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuTrigger } from '@/shared/ui/dropdown-menu';
import { Tooltip } from '@/shared/ui/tooltip';
import type { AttentionKind, Locale, SessionDetails, SessionInfo } from '@/types';
import type { MessageKey } from '@/shared/lib/i18n';
import { TerminalView } from './terminal-view';

interface WorkspaceShellProps {
  backendAddress: string;
  sessions: SessionInfo[];
  active?: SessionInfo;
  activeId: string | null;
  attention: Record<string, AttentionKind>;
  terminalFontSize: number;
  detailsOpen: boolean;
  connected: boolean;
  workspaceEnabled: boolean;
  locale: Locale;
  t: (key: MessageKey) => string;
  onSelect: (id: string) => void;
  onStatus: (session: SessionInfo) => void;
  onAttention: (session: SessionInfo, kind: AttentionKind) => void;
  onNew: () => void;
  onDetails: () => void;
  onRestart: () => void;
  onClose: () => void;
}

export function WorkspaceShell(props: WorkspaceShellProps) {
  const { active, sessions, t } = props;
  const [details, setDetails] = useState<SessionDetails | null>(null);

  useEffect(() => {
    if (!props.detailsOpen || !active) return;
    setDetails(null);
    const load = () => void sessionApi.sessionDetails(props.backendAddress, active.id).then(setDetails).catch(() => setDetails(null));
    load();
    const timer = window.setInterval(load, 5_000);
    return () => window.clearInterval(timer);
  }, [active?.id, props.backendAddress, props.detailsOpen]);

  return (
    <main className="workspace-surface ml-1.5 mt-1.5 flex min-w-0 flex-1 flex-col overflow-hidden rounded-tl-xl border border-border/60 bg-surface text-foreground max-[760px]:ml-1 max-[760px]:mt-1 max-[760px]:rounded-tl-lg">
      {sessions.length > 0 && (
        <div className="flex h-11 shrink-0 items-center gap-1 border-b border-border px-2">
          <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
            {sessions.map(session => (
              <button
                key={session.id}
                data-session-tab={session.id}
                data-active={session.id === props.activeId}
                data-attention={props.attention[session.id] || undefined}
                className={cn(
                  'flex h-8 min-w-32 max-w-56 shrink-0 items-center gap-1.5 rounded-lg px-2.5 text-xs text-muted-foreground hover:bg-accent',
                  session.id === props.activeId && 'bg-surface-raised text-foreground',
                )}
                onClick={() => props.onSelect(session.id)}
              >
                <AgentIcon agent={session.agent} className="size-5 rounded" />
                <span className="min-w-0 flex-1 truncate text-left">{session.title}</span>
                {props.attention[session.id] === 'input' ? (
                  <Bell className="session-signal session-signal-input" />
                ) : props.attention[session.id] === 'completed' ? (
                  <CircleCheck className="session-signal session-signal-completed" />
                ) : (
                  <span
                    className={cn(
                      'size-1.5 rounded-full bg-muted-foreground',
                      session.status === 'running' && 'bg-emerald-500',
                      (session.status === 'error' || session.status === 'exited') && 'bg-destructive',
                    )}
                  />
                )}
              </button>
            ))}
          </div>
          {active && (
            <div className="stage-actions flex shrink-0 items-center gap-0.5 border-l border-border pl-1.5">
              <DropdownMenu
                open={props.detailsOpen}
                onOpenChange={open => {
                  if (open !== props.detailsOpen) props.onDetails();
                }}
              >
                <Tooltip label={t('details')}>
                  <DropdownMenuTrigger asChild>
                    <Button variant="ghost" size="icon" aria-label={t('details')}>
                      <Info />
                    </Button>
                  </DropdownMenuTrigger>
                </Tooltip>
                <DropdownMenuContent align="end" className="session-details-popup w-[min(360px,calc(100vw-32px))] p-4">
                  <div className="mb-3 flex items-center gap-2">
                    <AgentIcon agent={active.agent} className="size-6" />
                    <div className="min-w-0">
                      <strong className="block truncate text-sm">{active.title}</strong>
                      <span className="text-xs text-muted-foreground">{active.agent === 'codex' ? 'Codex' : 'Claude Code'}</span>
                    </div>
                  </div>
                  <dl className="session-details-grid">
                    <Detail label={t('status')} value={t(active.status)} />
                    <Detail label={t('provider')} value={details?.provider_name || details?.provider_id || '-'} />
                    <Detail label={t('profile')} value={details?.profile_id || '-'} />
                    <Detail label={t('model')} value={details?.model || '-'} />
                    <Detail label={t('inputTokens')} value={formatCount(details?.prompt_tokens)} />
                    <Detail label={t('outputTokens')} value={formatCount(details?.completion_tokens)} />
                    <Detail label={t('cacheRead')} value={formatCount(details?.cache_read_tokens)} />
                    <Detail label={t('cacheCreate')} value={formatCount(details?.cache_creation_tokens)} />
                    <Detail label={t('messageCount')} value={formatCount(details?.message_count)} />
                    <Detail label={t('created')} value={new Date(active.created_at_ms).toLocaleString(props.locale)} />
                  </dl>
                  <Detail label={t('directory')} value={active.cwd} wide />
                  {active.error && (
                    <div className="mt-3 flex gap-2 rounded-md bg-destructive/10 p-3 text-xs text-destructive">
                      <CircleAlert className="size-4 shrink-0" />
                      <div>
                        <strong className="block font-medium">{t('sessionFailed')}</strong>
                        <span className="mt-0.5 block text-muted-foreground">{t('sessionFailedHint')}</span>
                      </div>
                    </div>
                  )}
                </DropdownMenuContent>
              </DropdownMenu>
              <Tooltip label={t('restart')}>
                <Button variant="ghost" size="icon" aria-label={t('restart')} disabled={active.status === 'starting'} onClick={props.onRestart}>
                  <RefreshCw />
                </Button>
              </Tooltip>
              <Tooltip label={t('close')}>
                <Button variant="destructive" size="icon" aria-label={t('close')} onClick={props.onClose}>
                  <X />
                </Button>
              </Tooltip>
            </div>
          )}
        </div>
      )}

      {!props.workspaceEnabled ? (
        <div className="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden">
          <div className="max-w-md px-8 text-center">
            <CircleAlert className="mx-auto size-8 text-muted-foreground" />
            <p className="mt-3 text-sm text-muted-foreground">{t('workspaceCapabilityUnavailable')}</p>
          </div>
        </div>
      ) : active ? (
        <>
          <div className="relative min-h-0 flex-1 overflow-hidden bg-[#0b0f12]">
            <div className="terminal-stack terminal-surface">
              {sessions.map(session => (
                <TerminalView
                  key={session.id}
                  backendAddress={props.backendAddress}
                  session={session}
                  active={session.id === props.activeId}
                  fontSize={props.terminalFontSize}
                  t={props.t}
                  onStatus={props.onStatus}
                  onAttention={props.onAttention}
                />
              ))}
            </div>
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
            <Button variant="secondary" className="mt-5" onClick={props.onNew} disabled={!props.workspaceEnabled}>
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

function Detail({ label, value, wide = false }: { label: string; value: string; wide?: boolean }) {
  return (
    <div className={wide ? 'mt-3 border-t border-border pt-3' : ''}>
      <dt className="mb-1 text-[11px] text-muted-foreground">{label}</dt>
      <dd className="m-0 break-words text-xs text-foreground">{value}</dd>
    </div>
  );
}

function formatCount(value?: number) {
  return value === undefined ? '-' : new Intl.NumberFormat().format(value);
}
