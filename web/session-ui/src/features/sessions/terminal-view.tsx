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
  onStatus: (session: SessionInfo) => void;
}

export function TerminalView({ backendAddress, session, active, onStatus }: TerminalViewProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const activeRef = useRef(active);
  const statusRef = useRef(onStatus);

  activeRef.current = active;
  statusRef.current = onStatus;

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const terminal = new Terminal({
      cursorBlink: true,
      scrollback: 10_000,
      fontFamily: '"Maple Mono NF NL", "Maple Mono NF CN", monospace',
      fontSize: 16,
      lineHeight: 1.2,
      theme: {
        background: '#0b0f12',
        foreground: '#e5e7eb',
        cursor: '#d7e3ff',
        selectionBackground: '#345a9c88',
      },
    });
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.open(host);
    let socket: WebSocket | null = null;
    let reconnectTimer: number | null = null;
    let disposed = false;

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
          terminal.write(new Uint8Array(event.data));
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
    const resizeDisposable = terminal.onResize(size => {
      if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ type: 'resize', rows: size.rows, cols: size.cols }));
    });
    const observer = new ResizeObserver(() => {
      if (activeRef.current) requestAnimationFrame(() => fit.fit());
    });
    observer.observe(host);
    connect();

    return () => {
      disposed = true;
      if (reconnectTimer !== null) window.clearTimeout(reconnectTimer);
      socket?.close();
      observer.disconnect();
      dataDisposable.dispose();
      resizeDisposable.dispose();
      terminal.dispose();
    };
  }, [backendAddress, session.id]);

  return <div ref={hostRef} className={cn('terminal-host', !active && 'invisible pointer-events-none')} aria-hidden={!active} />;
}
