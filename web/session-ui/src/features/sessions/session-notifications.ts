import { getCurrentWindow, UserAttentionType } from '@tauri-apps/api/window';
import { desktopShell } from '@/features/desktop/desktop-shell';
import type { AttentionKind, Locale, SessionInfo } from '@/types';

export function appHasFocus() {
  return document.visibilityState === 'visible' && document.hasFocus();
}

export async function notifySession(session: SessionInfo, kind: AttentionKind, locale: Locale) {
  if (appHasFocus()) return;

  const title = kind === 'input' ? (locale === 'zh-CN' ? '会话等待操作' : 'Session needs attention') : locale === 'zh-CN' ? '会话已结束' : 'Session finished';
  const body = `${session.agent === 'claude' ? 'Claude Code' : 'Codex'} · ${session.title}`;

  if ('Notification' in window) {
    try {
      const permission = Notification.permission === 'default' ? await Notification.requestPermission() : Notification.permission;
      if (permission === 'granted') new Notification(title, { body, icon: '/akiron.svg', tag: `akmux-${kind}-${session.id}` });
    } catch {
      // The WebView notification API is optional; taskbar attention remains available.
    }
  }

  if (desktopShell) {
    try {
      await getCurrentWindow().requestUserAttention(kind === 'input' ? UserAttentionType.Critical : UserAttentionType.Informational);
    } catch {
      // Ignore platforms that do not expose native attention requests.
    }
  }
}
