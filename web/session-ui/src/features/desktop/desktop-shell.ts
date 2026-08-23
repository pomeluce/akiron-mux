// The installed Desktop Client is GPUI. This bundle is now only the Embedded
// WebUI served by the daemon, so native-shell behavior must never be enabled.
export const desktopShell = false;

export function installDesktopInteractionGuards() {
  if (!desktopShell) return;

  document.addEventListener('contextmenu', event => event.preventDefault());
  document.addEventListener('dragstart', event => {
    const target = event.target instanceof Element ? event.target : null;
    if (target?.closest('a[href], img')) event.preventDefault();
  });
}
