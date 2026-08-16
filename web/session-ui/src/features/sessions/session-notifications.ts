import { getCurrentWindow, UserAttentionType } from '@tauri-apps/api/window';
import { isPermissionGranted, requestPermission, sendNotification } from '@tauri-apps/plugin-notification';
import { desktopShell } from '@/features/desktop/desktop-shell';
import type { AttentionKind, Locale, SessionInfo } from '@/types';

export function appHasFocus() {
  return document.visibilityState === 'visible' && document.hasFocus();
}

export async function notifySession(session: SessionInfo, kind: AttentionKind, locale: Locale) {
  if (appHasFocus()) return;

  const title = kind === 'input' ? (locale === 'zh-CN' ? '会话等待操作' : 'Session needs attention') : locale === 'zh-CN' ? '会话已结束' : 'Session finished';
  const body = `${session.agent === 'claude' ? 'Claude Code' : 'Codex'} · ${session.title}`;

  if (desktopShell) {
    try {
      let granted = await isPermissionGranted();
      if (!granted) granted = (await requestPermission()) === 'granted';
      if (granted) sendNotification({ title, body, icon: '/akiron.svg' });
    } catch {
      // Taskbar attention remains available if the native notification service fails.
    }
    try {
      await getCurrentWindow().requestUserAttention(kind === 'input' ? UserAttentionType.Critical : UserAttentionType.Informational);
    } catch {
      // Ignore platforms that do not expose native attention requests.
    }
  } else if ('Notification' in window) {
    try {
      const permission = Notification.permission === 'default' ? await Notification.requestPermission() : Notification.permission;
      if (permission === 'granted') new Notification(title, { body, icon: '/akiron.svg', tag: `akmux-${kind}-${session.id}` });
    } catch {
      // Browser notifications are optional outside the desktop shell.
    }
  }
}
