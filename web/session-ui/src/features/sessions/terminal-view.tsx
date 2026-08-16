import { FitAddon } from '@xterm/addon-fit';
import { Terminal } from '@xterm/xterm';
import { useEffect, useRef } from 'react';
import { websocketUrl } from '@/shared/lib/api';
import { cn } from '@/shared/lib/utils';
import type { SessionInfo } from '@/types';

interface TerminalViewProps {
  backendAddress: string;
  session: SessionInfo;
  active: boolean;
  fontSize: number;
  onStatus: (session: SessionInfo) => void;
  onAttention: (session: SessionInfo) => void;
}

const ATTENTION_PATTERN = /(?:allow|approve|approval|permission|confirm|proceed|continue|yes\s*\/\s*no|\(y\/n\)|授权|批准|允许|确认|继续)/i;

export function TerminalView({ backendAddress, session, active, fontSize, onStatus, onAttention }: TerminalViewProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const statusRef = useRef(onStatus);
  const attentionRef = useRef(onAttention);
  const terminalRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const syncingViewportRef = useRef(false);

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
    let lastAttentionAt = 0;

    const signalAttention = () => {
      const now = Date.now();
      if (now - lastAttentionAt < 2_000) return;
      lastAttentionAt = now;
      outputTail = '';
      attentionRef.current(session);
    };

    const connect = () => {
      if (disposed) return;
      socket = new WebSocket(websocketUrl(backendAddress, `/api/sessions/${encodeURIComponent(session.id)}/terminal`));
      socket.binaryType = 'arraybuffer';
      socket.addEventListener('open', () => {
        reconnectTimer = null;
        requestAnimationFrame(() => fit.fit());
      });
      socket.addEventListener('message', event => {
        if (event.data instanceof ArrayBuffer) {
          const bytes = new Uint8Array(event.data);
          terminal.write(bytes);
          const text = new TextDecoder().decode(bytes).replace(/\x1b\[[0-?]*[ -\/]*[@-~]/g, '');
          outputTail = `${outputTail}${text}`.slice(-1_200);
          if (ATTENTION_PATTERN.test(outputTail)) signalAttention();
          return;
        }
        const message = JSON.parse(event.data) as { type: string; session?: SessionInfo };
        if (message.type === 'status' && message.session) statusRef.current(message.session);
      });
      socket.addEventListener('close', () => {
        if (!disposed && reconnectTimer === null) reconnectTimer = window.setTimeout(connect, 1_200);
      });
    };

    const dataDisposable = terminal.onData(data => {
      if (socket?.readyState === WebSocket.OPEN) socket.send(new TextEncoder().encode(data));
    });
    const bellDisposable = terminal.onBell(signalAttention);
    const resizeDisposable = terminal.onResize(size => {
      if (syncingViewportRef.current) return;
      if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ type: 'resize', rows: size.rows, cols: size.cols }));
    });
    const observer = new ResizeObserver(() => requestAnimationFrame(() => fit.fit()));
    observer.observe(host);
    connect();

    return () => {
      disposed = true;
      if (reconnectTimer !== null) window.clearTimeout(reconnectTimer);
      socket?.close();
      observer.disconnect();
      dataDisposable.dispose();
      resizeDisposable.dispose();
      bellDisposable.dispose();
      terminalRef.current = null;
      fitRef.current = null;
      terminal.dispose();
    };
  }, [backendAddress, session.id]);

  return <div ref={hostRef} className={cn('terminal-host', !active && 'invisible pointer-events-none')} aria-hidden={!active} />;
}
