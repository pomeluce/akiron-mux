import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { Terminal } from '@xterm/xterm';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useEffect, useRef, useState } from 'react';
import { desktopShell } from '@/features/desktop/desktop-shell';
import type { MessageKey } from '@/shared/lib/i18n';
import { Button } from '@/shared/ui/button';
import { cn } from '@/shared/lib/utils';
import type { AttentionKind, SessionInfo } from '@/types';
import { TerminalConnection, type TerminalConnectionPhase, type TerminalLeaseState } from './terminal-connection';

interface TerminalViewProps {
  backendAddress: string;
  backendKey: string;
  session: SessionInfo;
  active: boolean;
  focusRequest: number;
  fontSize: number;
  t: (key: MessageKey) => string;
  onStatus: (session: SessionInfo) => void;
  onAttention: (session: SessionInfo, kind: AttentionKind) => void;
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

export function TerminalView({ backendAddress, backendKey, session, active, focusRequest, fontSize, t, onStatus, onAttention }: TerminalViewProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const statusRef = useRef(onStatus);
  const attentionRef = useRef(onAttention);
  const terminalRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const syncingViewportRef = useRef(false);
  const connectionRef = useRef<TerminalConnection | null>(null);
  const [lease, setLease] = useState<TerminalLeaseState | null>(null);
  const [canWrite, setCanWrite] = useState(false);
  const [connectionPhase, setConnectionPhase] = useState<TerminalConnectionPhase>('connecting');

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
        // becomes active. A real one-row resize pulse rebuilds the scroll model.
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
    const terminal = terminalRef.current;
    if (!active || !focusRequest || !terminal) return;
    const frame = requestAnimationFrame(() => {
      if (terminalRef.current === terminal) terminal.focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [active, focusRequest]);

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
    let terminalControlTail = '';
    let terminalAttentionReady = false;
    let lastResizeAt = 0;

    const sendResize = (rows = terminal.rows, cols = terminal.cols) => {
      lastResizeAt = performance.now();
      connectionRef.current?.resize(rows, cols);
    };
    const emitAttention = (kind: AttentionKind) => {
      attentionRef.current(session, kind);
    };
    const fitAndResize = () => {
      fit.fit();
      sendResize();
      requestAnimationFrame(() => {
        if (terminalRef.current !== terminal) return;
        fit.fit();
        sendResize();
      });
    };

    const connection = new TerminalConnection({
      backendAddress,
      backendKey,
      session,
      callbacks: {
        onOutput: (bytes, replay) => {
          if (replay?.replace) {
            terminal.reset();
            terminal.clear();
          }
          terminal.write(bytes);
          const terminalControls = `${terminalControlTail}${new TextDecoder().decode(bytes)}`;
          terminalControlTail = terminalControls.slice(-3);
          if (!replay && terminalAttentionReady && session.agent === 'codex' && terminalControls.includes('\x1b]9;') && performance.now() - lastResizeAt >= 100) {
            emitAttention('input');
          }
        },
        onStatus: next => {
          terminalAttentionReady = true;
          statusRef.current(next);
        },
        onAttention: emitAttention,
        onLease: (nextLease, nextCanWrite) => {
          setLease(nextLease);
          setCanWrite(nextCanWrite);
        },
        onPhase: phase => {
          setConnectionPhase(phase);
          if (phase === 'open') fitAndResize();
        },
        onProtocolError: message => console.warn(message),
      },
    });
    connectionRef.current = connection;

    const dataDisposable = terminal.onData(data => {
      connection.sendInput(data);
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
    connection.start();

    return () => {
      connection.dispose();
      connectionRef.current = null;
      observer.disconnect();
      dataDisposable.dispose();
      resizeDisposable.dispose();
      terminalRef.current = null;
      fitRef.current = null;
      terminal.dispose();
    };
  }, [backendAddress, backendKey, session.id]);

  return (
    <div className={cn('terminal-view-shell', !active && 'invisible pointer-events-none')} aria-hidden={!active} data-connection-phase={connectionPhase}>
      <div ref={hostRef} className="terminal-host" aria-hidden={!active} />
      {lease && !canWrite && (
        <div className="terminal-lease-banner">
          <span>
            {t('readOnly')}
            {lease.controller_device_name ? ` · ${t('controlledBy')} ${lease.controller_device_name}` : ''}
          </span>
          <Button size="sm" variant="secondary" onClick={() => connectionRef.current?.takeControl()}>
            {t('takeControl')}
          </Button>
        </div>
      )}
    </div>
  );
}
