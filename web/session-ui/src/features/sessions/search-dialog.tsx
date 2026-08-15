import { Search } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { AgentIcon } from '@/shared/components/agent-icon';
import { basename } from '@/shared/lib/utils';
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/shared/ui/dialog';
import type { HistoryItem, WorkspaceResponse } from '@/types';
import type { MessageKey } from '@/shared/lib/i18n';

export function SearchDialog({
  open,
  workspace,
  t,
  onOpenChange,
  onResume,
}: {
  open: boolean;
  workspace: WorkspaceResponse;
  t: (key: MessageKey) => string;
  onOpenChange: (open: boolean) => void;
  onResume: (item: HistoryItem) => void;
}) {
  const [query, setQuery] = useState('');
  useEffect(() => {
    if (open) setQuery('');
  }, [open]);
  const allItems = useMemo(() => {
    const items = [...workspace.projects.flatMap(group => group.history), ...workspace.general.flatMap(group => group.items), ...workspace.other.flatMap(group => group.items)];
    return [...new Map(items.map(item => [`${item.agent}:${item.id}`, item])).values()];
  }, [workspace]);
  const results = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    if (!normalized) return allItems.slice(0, 40);
    return allItems.filter(item => `${item.title} ${item.cwd} ${item.agent}`.toLowerCase().includes(normalized)).slice(0, 80);
  }, [allItems, query]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent id="search-popover" className="top-[18%] w-[min(640px,calc(100vw-28px))] translate-y-0">
        <DialogHeader>
          <DialogTitle>{t('search')}</DialogTitle>
        </DialogHeader>
        <div className="flex items-center gap-2 border-b border-border px-5 py-3">
          <Search className="size-5 text-muted-foreground" />
          <input
            autoFocus
            className="h-10 min-w-0 flex-1 bg-transparent text-base outline-none"
            value={query}
            onChange={event => setQuery(event.target.value)}
            placeholder={t('searchHint')}
          />
        </div>
        <div className="max-h-[52vh] overflow-y-auto p-2">
          {results.length ? (
            results.map(item => (
              <button
                key={`${item.agent}:${item.id}`}
                className="flex min-h-12 w-full items-center gap-3 rounded-lg px-3 text-left hover:bg-accent"
                onClick={() => {
                  onResume(item);
                  onOpenChange(false);
                }}
              >
                <AgentIcon agent={item.agent} />
                <span className="min-w-0 flex-1">
                  <strong className="block truncate text-sm font-medium">{item.title}</strong>
                  <small className="block truncate text-xs text-muted-foreground">
                    {basename(item.cwd)} · {item.cwd}
                  </small>
                </span>
              </button>
            ))
          ) : (
            <div className="p-8 text-center text-sm text-muted-foreground">{t('noResults')}</div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
