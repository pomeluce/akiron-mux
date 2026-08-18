import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { Terminal } from '@xterm/xterm';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useEffect, useRef, useState } from 'react';
import { sessionApi, websocketUrl } from '@/shared/lib/api';
import { currentDesktopBackend } from '@/features/backends/desktop-backend';
import { desktopShell } from '@/features/desktop/desktop-shell';
import type { MessageKey } from '@/shared/lib/i18n';
import { Button } from '@/shared/ui/button';
import { cn } from '@/shared/lib/utils';
import type { AttentionKind, SessionInfo } from '@/types';

interface TerminalViewProps {
  backendAddress: string;
  session: SessionInfo;
  active: boolean;
  fontSize: number;
  t: (key: MessageKey) => string;
  onStatus: (session: SessionInfo) => void;
  onAttention: (session: SessionInfo, kind: AttentionKind) => void;
}

interface LeaseState {
  version: number;
  controller_device_name?: string;
}

function openExternalUrl(value: string) {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return;
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') return;
  if (desktopShell) {
    void openUrl(url.toString()).catch(() => undefined);
    return;
  }
  window.open(url.toString(), '_blank', 'noopener,noreferrer');
}

export function TerminalView({ backendAddress, session, active, fontSize, t, onStatus, onAttention }: TerminalViewProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const statusRef = useRef(onStatus);
  const attentionRef = useRef(onAttention);
  const terminalRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const syncingViewportRef = useRef(false);
  const socketRef = useRef<WebSocket | null>(null);
  const [lease, setLease] = useState<LeaseState | null>(null);
  const [canWrite, setCanWrite] = useState(true);

  statusRef.current = onStatus;
  attentionRef.current = onAttention;

  useEffect(() => {
    const terminal = terminalRef.current;
    const fit = fitRef.current;
    if (!terminal || !fit) return;
    terminal.options.fontSize = fontSize;
    if (active) {
      requestAnimationFrame(() => {
        if (terminalRef.current !== terminal || fitRef.current !== fit) return;
        fit.fit();
        // xterm 6 can leave its custom viewport stale when a visibility-hidden terminal
        // becomes active. A real one-row resize pulse rebuilds the scroll model, matching
        // the redraw that previously happened only after the sidebar was dragged.
        const { cols, rows } = terminal;
        const viewportY = terminal.buffer.active.viewportY;
        syncingViewportRef.current = true;
        try {
          terminal.resize(cols, rows + 1);
          terminal.resize(cols, rows);
        } finally {
          syncingViewportRef.current = false;
        }
        terminal.scrollToLine(Math.min(viewportY, terminal.buffer.active.baseY));
        terminal.refresh(0, terminal.rows - 1);
      });
    }
  }, [active, fontSize]);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const terminal = new Terminal({
      cursorBlink: true,
      scrollback: 10_000,
      fontFamily: '"Maple Mono NF NL", "Maple Mono NF CN", monospace',
      fontSize,
      lineHeight: 1.2,
      theme: {
        background: '#0b0f12',
        foreground: '#e5e7eb',
        cursor: '#d7e3ff',
        selectionBackground: '#345a9c88',
      },
      linkHandler: {
        activate: (_event, url) => openExternalUrl(url),
        allowNonHttpProtocols: false,
      },
    });
    const fit = new FitAddon();
    terminalRef.current = terminal;
    fitRef.current = fit;
    terminal.loadAddon(fit);
    terminal.loadAddon(new WebLinksAddon((_event, url) => openExternalUrl(url)));
    terminal.open(host);
    requestAnimationFrame(() => fit.fit());
    let socket: WebSocket | null = null;
    let reconnectTimer: number | null = null;
    let disposed = false;
    let lastResizeAt = 0;
    let terminalControlTail = '';
    let lastAttention: { kind: AttentionKind; at: number } | null = null;
    let reconnectAttempts = 0;
    const recoveryKey = `akmux.lease-recovery:${backendAddress}:${session.id}`;

    const sendResize = (rows = terminal.rows, cols = terminal.cols) => {
      lastResizeAt = performance.now();
      if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ type: 'resize', rows, cols }));
    };

    const emitAttention = (kind: AttentionKind) => {
      const now = performance.now();
      if (lastAttention?.kind === kind && now - lastAttention.at < 2_000) return;
      lastAttention = { kind, at: now };
      attentionRef.current(session, kind);
    };

    const scheduleReconnect = () => {
      if (disposed || reconnectTimer !== null) return;
      const delay = Math.min(1_200 * 2 ** reconnectAttempts, 30_000);
      reconnectAttempts += 1;
      reconnectTimer = window.setTimeout(() => void connect(), delay);
    };

    const connect = async () => {
      if (disposed) return;
      const url = new URL(websocketUrl(backendAddress, `/api/sessions/${encodeURIComponent(session.id)}/terminal`));
      const backend = currentDesktopBackend();
      if (backend?.kind === 'remote') {
        try {
          const ticket = await sessionApi.wsTicket(backendAddress, session.id);
          url.searchParams.set('ticket', ticket.ticket);
        } catch {
          scheduleReconnect();
          return;
        }
      }
      if (disposed) return;
      const recoveryCredential = sessionStorage.getItem(recoveryKey);
      socket = new WebSocket(url);
      socketRef.current = socket;
      socket.binaryType = 'arraybuffer';
      socket.addEventListener('open', () => {
        reconnectTimer = null;
        reconnectAttempts = 0;
        if (recoveryCredential) socket?.send(JSON.stringify({ type: 'recover-control', credential: recoveryCredential }));
        fit.fit();
        sendResize();
        requestAnimationFrame(() => {
          fit.fit();
          sendResize();
        });
      });
      socket.addEventListener('message', event => {
        if (event.data instanceof ArrayBuffer) {
          const bytes = new Uint8Array(event.data);
          terminal.write(bytes);
          const terminalControls = `${terminalControlTail}${new TextDecoder().decode(bytes)}`;
          terminalControlTail = terminalControls.slice(-3);
          if (session.agent === 'codex' && terminalControls.includes('\x1b]9;') && performance.now() - lastResizeAt >= 100) {
            emitAttention('input');
          }
          return;
        }
        const message = JSON.parse(event.data) as {
          type: string;
          session?: SessionInfo;
          credential?: string;
          lease?: LeaseState;
          can_write?: boolean;
          kind?: AttentionKind;
        };
        if (message.type === 'status' && message.session) statusRef.current(message.session);
        if (message.type === 'attention' && message.kind) emitAttention(message.kind);
        if (message.type === 'lease' && message.lease) {
          setLease(message.lease);
          setCanWrite(Boolean(message.can_write));
        }
        if (message.type === 'lease-recovery' && message.credential) {
          sessionStorage.setItem(recoveryKey, message.credential);
          setCanWrite(true);
        }
        if (message.type === 'authorization-revoked') sessionStorage.removeItem(recoveryKey);
      });
      socket.addEventListener('close', () => {
        scheduleReconnect();
      });
    };

    const dataDisposable = terminal.onData(data => {
      if (socket?.readyState === WebSocket.OPEN) socket.send(new TextEncoder().encode(data));
    });
    terminal.attachCustomKeyEventHandler(event => {
      const copySelection = (event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'c' && terminal.hasSelection();
      if (!copySelection) return true;
      if (event.type === 'keydown') void navigator.clipboard.writeText(terminal.getSelection()).catch(() => undefined);
      return false;
    });
    const resizeDisposable = terminal.onResize(size => {
      if (syncingViewportRef.current) return;
      sendResize(size.rows, size.cols);
    });
    const observer = new ResizeObserver(() => requestAnimationFrame(() => fit.fit()));
    observer.observe(host);
    void connect();

    return () => {
      disposed = true;
      if (reconnectTimer !== null) window.clearTimeout(reconnectTimer);
      socket?.close();
      socketRef.current = null;
      observer.disconnect();
      dataDisposable.dispose();
      resizeDisposable.dispose();
      terminalRef.current = null;
      fitRef.current = null;
      terminal.dispose();
    };
  }, [backendAddress, session.id]);

  const takeControl = () => {
    if (!lease || socketRef.current?.readyState !== WebSocket.OPEN) return;
    socketRef.current.send(JSON.stringify({ type: 'take-control', expected_version: lease.version }));
  };

  return (
    <div className={cn('terminal-view-shell', !active && 'invisible pointer-events-none')} aria-hidden={!active}>
      <div ref={hostRef} className="terminal-host" aria-hidden={!active} />
      {lease && !canWrite && (
        <div className="terminal-lease-banner">
          <span>
            {t('readOnly')}
            {lease.controller_device_name ? ` · ${t('controlledBy')} ${lease.controller_device_name}` : ''}
          </span>
          <Button size="sm" variant="secondary" onClick={takeControl}>
            {t('takeControl')}
          </Button>
        </div>
      )}
    </div>
  );
}
