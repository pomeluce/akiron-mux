import { FitAddon } from '@xterm/addon-fit';
import { Terminal } from '@xterm/xterm';
import { useEffect, useRef, useState } from 'react';
import { sessionApi, websocketUrl } from '@/shared/lib/api';
import { currentDesktopBackend } from '@/features/backends/desktop-backend';
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

const PERMISSION_PROMPT_PATTERN = /(?:allow|approve|approval|permission|confirm|proceed|yes\s*\/\s*no|\(y\/n\)|授权|批准|允许|确认|是否继续|继续执行)/i;

interface LeaseState {
  version: number;
  controller_device_name?: string;
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
    });
    const fit = new FitAddon();
    terminalRef.current = terminal;
    fitRef.current = fit;
    terminal.loadAddon(fit);
    terminal.open(host);
    requestAnimationFrame(() => fit.fit());
    let socket: WebSocket | null = null;
    let reconnectTimer: number | null = null;
    let disposed = false;
    let outputTail = '';
    let lastSignal: AttentionKind | null = null;
    let lastResizeAt = 0;
    let outputSinceLastResize = false;
    let reconnectAttempts = 0;
    const recoveryKey = `akmux.lease-recovery:${backendAddress}:${session.id}`;

    const terminalTail = () => {
      const buffer = terminal.buffer.active;
      const start = Math.max(0, buffer.baseY + buffer.cursorY - 12);
      const end = Math.min(buffer.length, buffer.baseY + buffer.cursorY + 1);
      let value = '';
      for (let index = start; index < end; index += 1) value += `${buffer.getLine(index)?.translateToString(true) || ''}\n`;
      return value.slice(-1_600);
    };

    const signalAttention = (kind: AttentionKind) => {
      if (lastSignal === kind) return;
      lastSignal = kind;
      attentionRef.current(session, kind);
    };

    const sendResize = (rows = terminal.rows, cols = terminal.cols) => {
      lastResizeAt = performance.now();
      outputSinceLastResize = false;
      if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ type: 'resize', rows, cols }));
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
          const text = new TextDecoder().decode(bytes).replace(/\x1b\[[0-?]*[ -\/]*[@-~]/g, '');
          outputTail = `${outputTail}${text}`.slice(-1_200);
          if (text.replace(/\x07/g, '').trim()) outputSinceLastResize = true;
          if (bytes.includes(7)) {
            const bellOnly = text.replace(/\x07/g, '').trim().length === 0;
            if (!bellOnly || outputSinceLastResize || performance.now() - lastResizeAt >= 750) {
              const tail = `${outputTail}\n${terminalTail()}`;
              signalAttention(PERMISSION_PROMPT_PATTERN.test(tail) ? 'input' : 'completed');
            }
          }
          return;
        }
        const message = JSON.parse(event.data) as {
          type: string;
          session?: SessionInfo;
          credential?: string;
          lease?: LeaseState;
          can_write?: boolean;
        };
        if (message.type === 'status' && message.session) statusRef.current(message.session);
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
      lastSignal = null;
      outputTail = '';
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
