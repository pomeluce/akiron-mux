import { useEffect, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { desktopShell } from '@/features/desktop/desktop-shell';
import { cn } from '@/shared/lib/utils';
import type { MessageKey } from '@/shared/lib/i18n';

const appWindow = desktopShell ? getCurrentWindow() : null;

export function toggleDesktopMaximize() {
  return appWindow?.toggleMaximize();
}

export function WindowControls({ t }: { t: (key: MessageKey) => string }) {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!appWindow) return;

    let unlisten: (() => void) | undefined;
    const update = () => void appWindow.isMaximized().then(setMaximized);
    update();
    void appWindow.onResized(update).then(dispose => {
      unlisten = dispose;
    });
    return () => unlisten?.();
  }, []);

  if (!appWindow) return null;

  return (
    <div className="window-controls ml-1 flex self-stretch" data-window-controls>
      <WindowButton label={t('minimize')} onClick={() => void appWindow.minimize()}>
        <span className="window-control-icon window-control-icon-minimize" />
      </WindowButton>
      <WindowButton
        label={t(maximized ? 'restore' : 'maximize')}
        onClick={() => {
          void appWindow.toggleMaximize().then(() => appWindow.isMaximized()).then(setMaximized);
        }}
      >
        <span className={`window-control-icon ${maximized ? 'window-control-icon-restore' : 'window-control-icon-maximize'}`} />
      </WindowButton>
      <WindowButton label={t('close')} close onClick={() => void appWindow.close()}>
        <span className="window-control-icon window-control-icon-close" />
      </WindowButton>
    </div>
  );
}

function WindowButton(props: { label: string; close?: boolean; children: React.ReactNode; onClick: () => void }) {
  return (
    <button
      type="button"
      className={cn(
        'grid w-11 place-items-center text-foreground/75 transition-colors hover:bg-foreground/10 hover:text-foreground',
        props.close && 'hover:bg-[#c42b1c] hover:text-white',
      )}
      aria-label={props.label}
      title={props.label}
      onClick={props.onClick}
    >
      {props.children}
    </button>
  );
}
