import type { AttentionKind, Locale, SessionInfo } from '@/types';

export function appHasFocus() {
  return document.visibilityState === 'visible' && document.hasFocus();
}

export async function notifySession(session: SessionInfo, kind: AttentionKind, locale: Locale) {
  if (appHasFocus()) return;

  const title = kind === 'input' ? (locale === 'zh-CN' ? '会话等待操作' : 'Session needs attention') : locale === 'zh-CN' ? '回答已完成' : 'Response completed';
  const body = `${session.agent === 'claude' ? 'Claude Code' : 'Codex'} · ${session.title}`;

  if ('Notification' in window) {
    try {
      const permission = Notification.permission === 'default' ? await Notification.requestPermission() : Notification.permission;
      if (permission === 'granted') new Notification(title, { body, icon: '/akiron.svg', tag: `akmux-${kind}-${session.id}` });
    } catch {
      // Browser notifications are optional outside the desktop shell.
    }
  }
}
