export const desktopShell =
  window.location.protocol === 'tauri:' || window.location.hostname === 'tauri.localhost' || '__TAURI_INTERNALS__' in window;

export function installDesktopInteractionGuards() {
  if (!desktopShell) return;

  document.addEventListener('contextmenu', event => event.preventDefault());
  document.addEventListener('dragstart', event => {
    const target = event.target instanceof Element ? event.target : null;
    if (target?.closest('a[href], img')) event.preventDefault();
  });
}
