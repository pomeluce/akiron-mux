import { currentDesktopBackend } from '@/features/backends/desktop-backend';
import { sessionApi, websocketUrl } from '@/shared/lib/api';
import type { AttentionKind, SessionInfo } from '@/types';

export type TerminalConnectionPhase = 'requesting-ticket' | 'connecting' | 'open' | 'reconnecting' | 'revoked' | 'disposed';

export interface TerminalLeaseState {
  version: number;
  controller_device_name: string | null;
}

interface TerminalConnectionCallbacks {
  onOutput: (bytes: Uint8Array, replay: { replace: boolean } | null) => void;
  onStatus: (session: SessionInfo) => void;
  onAttention: (kind: AttentionKind) => void;
  onLease: (lease: TerminalLeaseState, canWrite: boolean) => void;
  onPhase: (phase: TerminalConnectionPhase) => void;
  onProtocolError?: (message: string) => void;
}

interface TerminalConnectionOptions {
  backendAddress: string;
  backendKey: string;
  session: SessionInfo;
  callbacks: TerminalConnectionCallbacks;
}

const ATTENTION_MAX_DELIVERY_AGE_MS = 30_000;
const RECONNECT_BASE_DELAY_MS = 1_200;
const RECONNECT_MAX_DELAY_MS = 30_000;

export class TerminalConnection {
  private socket: WebSocket | null = null;
  private reconnectTimer: number | null = null;
  private reconnectAttempts = 0;
  private disposed = false;
  private revoked = false;
  private canWrite = false;
  private lease: TerminalLeaseState | null = null;
  private pendingReplay: { replace: boolean } | null = null;
  private pendingResize: { rows: number; cols: number } | null = null;
  private serverClockOffsetMs: number | null = null;
  private phase: TerminalConnectionPhase = 'connecting';
  private readonly recoveryKey: string;
  private readonly legacyRecoveryKey: string;

  constructor(private readonly options: TerminalConnectionOptions) {
    this.recoveryKey = `akmux.lease-recovery:${options.backendKey}:${options.session.id}`;
    this.legacyRecoveryKey = `akmux.lease-recovery:${options.backendAddress}:${options.session.id}`;
    this.migrateRecoveryCredential();
  }

  start() {
    void this.connect();
  }

  sendInput(data: string) {
    if (!this.canSendControlledData()) return false;
    this.socket!.send(new TextEncoder().encode(data));
    return true;
  }

  resize(rows: number, cols: number) {
    this.pendingResize = { rows, cols };
    if (!this.canSendControlledData()) return false;
    this.socket!.send(JSON.stringify({ type: 'resize', rows, cols }));
    return true;
  }

  takeControl() {
    if (!this.lease || this.socket?.readyState !== WebSocket.OPEN) return false;
    this.socket.send(JSON.stringify({ type: 'take-control', expected_version: this.lease.version }));
    return true;
  }

  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    if (this.reconnectTimer !== null) window.clearTimeout(this.reconnectTimer);
    this.reconnectTimer = null;
    this.socket?.close();
    this.socket = null;
    this.setPhase('disposed');
  }

  private async connect() {
    if (this.disposed || this.revoked) return;
    const url = new URL(websocketUrl(this.options.backendAddress, `/api/sessions/${encodeURIComponent(this.options.session.id)}/terminal`));
    const backend = currentDesktopBackend();
    if (backend?.kind === 'remote') {
      this.setPhase('requesting-ticket');
      try {
        const ticket = await sessionApi.wsTicket(this.options.backendAddress, this.options.session.id);
        url.searchParams.set('ticket', ticket.ticket);
      } catch {
        this.scheduleReconnect();
        return;
      }
    }
    if (this.disposed || this.revoked) return;
    this.pendingReplay = null;
    this.serverClockOffsetMs = null;
    this.setPhase(this.reconnectAttempts ? 'reconnecting' : 'connecting');

    let socket: WebSocket;
    try {
      socket = new WebSocket(url);
    } catch {
      this.scheduleReconnect();
      return;
    }
    this.socket = socket;
    socket.binaryType = 'arraybuffer';
    socket.addEventListener('open', () => {
      if (this.socket !== socket || this.disposed || this.revoked) return;
      this.reconnectAttempts = 0;
      this.setPhase('open');
      const credential = sessionStorage.getItem(this.recoveryKey);
      if (credential) socket.send(JSON.stringify({ type: 'recover-control', credential }));
      this.flushResize();
    });
    socket.addEventListener('message', event => {
      if (this.socket !== socket || this.disposed) return;
      this.handleMessage(event.data);
    });
    socket.addEventListener('close', () => {
      if (this.socket !== socket) return;
      this.socket = null;
      this.canWrite = false;
      if (!this.disposed && !this.revoked) this.scheduleReconnect();
    });
  }

  private handleMessage(data: unknown) {
    if (data instanceof ArrayBuffer) {
      const replay = this.pendingReplay;
      this.pendingReplay = null;
      this.options.callbacks.onOutput(new Uint8Array(data), replay);
      return;
    }
    if (typeof data !== 'string') return;
    let message: unknown;
    try {
      message = JSON.parse(data);
    } catch {
      this.protocolError('Server sent invalid terminal protocol JSON');
      return;
    }
    if (!isRecord(message) || typeof message.type !== 'string') {
      this.protocolError("Server terminal message requires a string 'type'");
      return;
    }
    switch (message.type) {
      case 'replay':
        if (typeof message.replace !== 'boolean') this.protocolError("Replay message requires boolean 'replace'");
        else this.pendingReplay = { replace: message.replace };
        return;
      case 'reset':
        this.pendingReplay = { replace: true };
        return;
      case 'status':
        if (!isSessionInfo(message.session)) {
          this.protocolError("Status message requires a valid 'session'");
          return;
        }
        if (typeof message.server_time_ms === 'number' && Number.isFinite(message.server_time_ms)) {
          this.serverClockOffsetMs = Date.now() - message.server_time_ms;
        }
        this.options.callbacks.onStatus(message.session);
        return;
      case 'attention':
        if ((message.kind !== 'input' && message.kind !== 'completed') || !isOptionalFiniteNumber(message.occurred_at_ms)) {
          this.protocolError("Attention message requires a valid 'kind' and optional 'occurred_at_ms'");
          return;
        }
        if (this.attentionIsFresh(message.occurred_at_ms)) this.options.callbacks.onAttention(message.kind);
        return;
      case 'lease':
        if (!isLeaseState(message.lease) || typeof message.can_write !== 'boolean') {
          this.protocolError("Lease message requires valid 'lease' and 'can_write' fields");
          return;
        }
        this.lease = message.lease;
        this.canWrite = message.can_write;
        if (!this.canWrite) sessionStorage.removeItem(this.recoveryKey);
        this.options.callbacks.onLease(message.lease, this.canWrite);
        this.flushResize();
        return;
      case 'lease-recovery':
        if (typeof message.credential !== 'string' || !message.credential) {
          this.protocolError("Lease recovery message requires a credential");
          return;
        }
        sessionStorage.setItem(this.recoveryKey, message.credential);
        return;
      case 'authorization-revoked':
        this.revoked = true;
        this.canWrite = false;
        sessionStorage.removeItem(this.recoveryKey);
        this.setPhase('revoked');
        this.socket?.close();
        return;
      case 'protocol-error':
        this.protocolError(typeof message.message === 'string' ? message.message : 'Terminal protocol error');
        return;
      default:
        console.debug(`Ignoring unknown terminal server message type '${message.type}'`);
    }
  }

  private scheduleReconnect() {
    if (this.disposed || this.revoked || this.reconnectTimer !== null) return;
    this.setPhase('reconnecting');
    const delay = Math.min(RECONNECT_BASE_DELAY_MS * 2 ** this.reconnectAttempts, RECONNECT_MAX_DELAY_MS);
    this.reconnectAttempts += 1;
    this.reconnectTimer = window.setTimeout(() => {
      this.reconnectTimer = null;
      void this.connect();
    }, delay);
  }

  private flushResize() {
    if (!this.pendingResize || !this.canSendControlledData()) return;
    const { rows, cols } = this.pendingResize;
    this.socket!.send(JSON.stringify({ type: 'resize', rows, cols }));
  }

  private canSendControlledData() {
    return this.canWrite && this.socket?.readyState === WebSocket.OPEN;
  }

  private attentionIsFresh(occurredAt: unknown) {
    if (typeof occurredAt !== 'number' || this.serverClockOffsetMs === null) return true;
    return Date.now() - (occurredAt + this.serverClockOffsetMs) <= ATTENTION_MAX_DELIVERY_AGE_MS;
  }

  private migrateRecoveryCredential() {
    if (this.recoveryKey === this.legacyRecoveryKey) return;
    if (!sessionStorage.getItem(this.recoveryKey)) {
      const legacy = sessionStorage.getItem(this.legacyRecoveryKey);
      if (legacy) sessionStorage.setItem(this.recoveryKey, legacy);
    }
    sessionStorage.removeItem(this.legacyRecoveryKey);
  }

  private setPhase(phase: TerminalConnectionPhase) {
    if (this.phase === phase) return;
    this.phase = phase;
    this.options.callbacks.onPhase(phase);
  }

  private protocolError(message: string) {
    this.options.callbacks.onProtocolError?.(message);
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function isLeaseState(value: unknown): value is TerminalLeaseState {
  return (
    isRecord(value) &&
    Number.isInteger(value.version) &&
    typeof value.version === 'number' &&
    value.version >= 0 &&
    (value.controller_device_name === null || typeof value.controller_device_name === 'string')
  );
}

function isSessionInfo(value: unknown): value is SessionInfo {
  return (
    isRecord(value) &&
    typeof value.id === 'string' &&
    (value.agent === 'claude' || value.agent === 'codex') &&
    typeof value.title === 'string' &&
    typeof value.cwd === 'string' &&
    (value.status === 'starting' || value.status === 'running' || value.status === 'exited' || value.status === 'error') &&
    typeof value.created_at_ms === 'number' &&
    Number.isFinite(value.created_at_ms) &&
    (value.exit_code === null || (typeof value.exit_code === 'number' && Number.isFinite(value.exit_code))) &&
    (value.error === null || typeof value.error === 'string') &&
    (value.native_session_id === null || typeof value.native_session_id === 'string')
  );
}

function isOptionalFiniteNumber(value: unknown) {
  return value === undefined || (typeof value === 'number' && Number.isFinite(value));
}
