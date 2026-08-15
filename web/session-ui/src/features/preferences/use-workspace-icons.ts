import { useState } from 'react';
import type { WorkspaceIconName } from '@/shared/components/workspace-icon';

const storageKey = 'akironmux-workspace-icons';

function initialIcons(): Record<string, WorkspaceIconName> {
  try {
    const value = JSON.parse(localStorage.getItem(storageKey) || '{}') as Record<string, WorkspaceIconName>;
    return value && typeof value === 'object' ? value : {};
  } catch {
    return {};
  }
}

export function useWorkspaceIcons() {
  const [icons, setIcons] = useState(initialIcons);
  const setIcon = (key: string, icon: WorkspaceIconName) => {
    setIcons(current => {
      const next = { ...current, [key]: icon };
      localStorage.setItem(storageKey, JSON.stringify(next));
      return next;
    });
  };
  return { icons, setIcon };
}
