import { useEffect, useMemo, useState } from 'react';
import { desktopShell } from '@/features/desktop/desktop-shell';
import { initialLocale } from '@/shared/lib/i18n';
import type { ClientPreferences, Locale, ThemeMode } from '@/types';

const ACRYLIC_TRANSPARENCY_KEY = 'akironmux-acrylic-transparency';
const TERMINAL_FONT_SIZE_KEY = 'akironmux-terminal-font-size';

function initialTheme(): ThemeMode {
  const value = localStorage.getItem('akironmux-theme');
  return value === 'light' || value === 'dark' || value === 'system' ? value : 'system';
}

function initialBackendAddress() {
  const saved = localStorage.getItem('akironmux-backend-address');
  if (saved) return saved;
  return desktopShell ? 'http://127.0.0.1:17321' : '';
}

function initialAcrylicTransparency() {
  const raw = localStorage.getItem(ACRYLIC_TRANSPARENCY_KEY);
  const saved = raw === null ? 20 : Number(raw);
  return Number.isFinite(saved) ? Math.min(Math.max(saved, 0), 100) : 20;
}

function initialTerminalFontSize() {
  const raw = localStorage.getItem(TERMINAL_FONT_SIZE_KEY);
  if (raw === null) return 12;
  const saved = Number(raw);
  return Number.isFinite(saved) ? Math.min(Math.max(saved, 10), 24) : 12;
}

export function usePreferences() {
  const [preferences, setPreferences] = useState<ClientPreferences>({
    locale: initialLocale(),
    theme: initialTheme(),
    acrylic: localStorage.getItem('akironmux-acrylic') !== 'false',
    acrylicStrength: initialAcrylicTransparency(),
    terminalFontSize: initialTerminalFontSize(),
    backendAddress: initialBackendAddress(),
  });
  const [systemDark, setSystemDark] = useState(() => matchMedia('(prefers-color-scheme: dark)').matches);

  useEffect(() => {
    const media = matchMedia('(prefers-color-scheme: dark)');
    const listener = (event: MediaQueryListEvent) => setSystemDark(event.matches);
    media.addEventListener('change', listener);
    return () => media.removeEventListener('change', listener);
  }, []);

  const resolvedTheme = preferences.theme === 'system' ? (systemDark ? 'dark' : 'light') : preferences.theme;

  useEffect(() => {
    document.documentElement.lang = preferences.locale;
    document.documentElement.dataset.theme = resolvedTheme;
    document.documentElement.dataset.acrylic = String(preferences.acrylic);
    document.documentElement.dataset.desktopShell = String(desktopShell);
    document.documentElement.style.setProperty('--acrylic-transparency', `${preferences.acrylicStrength}%`);
  }, [preferences, resolvedTheme]);

  const persist = (next: ClientPreferences) => {
    setPreferences(next);
    localStorage.setItem('akironmux-locale', next.locale);
    localStorage.setItem('akironmux-theme', next.theme);
    localStorage.setItem('akironmux-acrylic', String(next.acrylic));
    localStorage.setItem(ACRYLIC_TRANSPARENCY_KEY, String(next.acrylicStrength));
    localStorage.setItem(TERMINAL_FONT_SIZE_KEY, String(next.terminalFontSize));
    localStorage.removeItem('akironmux-acrylic-strength');
    localStorage.setItem('akironmux-backend-address', next.backendAddress);
  };

  return useMemo(
    () => ({
      preferences,
      resolvedTheme,
      persist,
      setLocale: (locale: Locale) => persist({ ...preferences, locale }),
    }),
    [preferences, resolvedTheme],
  );
}
