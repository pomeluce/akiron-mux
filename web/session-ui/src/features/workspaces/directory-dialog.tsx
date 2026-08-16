import { ChevronLeft, ChevronRight, Eye, EyeOff, Folder, Home } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { sessionApi } from '@/shared/lib/api';
import { Button } from '@/shared/ui/button';
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog';
import { Tooltip } from '@/shared/ui/tooltip';
import type { DirectoryListing } from '@/types';
import type { MessageKey } from '@/shared/lib/i18n';

interface DirectoryDialogProps {
  open: boolean;
  backendAddress: string;
  initialPath: string;
  t: (key: MessageKey) => string;
  onOpenChange: (open: boolean) => void;
  onChoose: (path: string) => void;
}

export function DirectoryDialog(props: DirectoryDialogProps) {
  const [listing, setListing] = useState<DirectoryListing | null>(null);
  const [path, setPath] = useState(props.initialPath);
  const [showHidden, setShowHidden] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const pathInputRef = useRef<HTMLInputElement>(null);

  const load = async (nextPath: string, hidden = showHidden) => {
    setLoading(true);
    setError(null);
    try {
      const next = await sessionApi.directories(props.backendAddress, nextPath, hidden);
      setListing(next);
      setPath(next.path);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (!props.open) return;
    setShowHidden(false);
    void load(props.initialPath, false);
  }, [props.open, props.initialPath]);

  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange}>
      <DialogContent
        className="directory-dialog w-[min(720px,calc(100vw-28px))]"
        onOpenAutoFocus={event => {
          event.preventDefault();
          requestAnimationFrame(() => pathInputRef.current?.focus({ preventScroll: true }));
        }}
      >
        <DialogHeader>
          <DialogTitle>{props.t('chooseDirectory')}</DialogTitle>
        </DialogHeader>
        <div className="p-5">
          <div className="flex items-center gap-1.5">
            <Tooltip label={props.t(showHidden ? 'hideHidden' : 'showHidden')}>
              <Button
                variant="ghost"
                size="icon"
                onClick={() => {
                  const next = !showHidden;
                  setShowHidden(next);
                  void load(listing?.path || path || props.initialPath, next);
                }}
              >
                {showHidden ? <EyeOff /> : <Eye />}
              </Button>
            </Tooltip>
            <Tooltip label={props.t('home')}>
              <Button variant="ghost" size="icon" disabled={!listing?.home} onClick={() => listing?.home && void load(listing.home)}>
                <Home />
              </Button>
            </Tooltip>
            <Tooltip label={props.t('parent')}>
              <Button variant="ghost" size="icon" disabled={!listing?.parent} onClick={() => listing?.parent && void load(listing.parent)}>
                <ChevronLeft />
              </Button>
            </Tooltip>
            <form
              className="min-w-0 flex-1"
              onSubmit={event => {
                event.preventDefault();
                void load(path);
              }}
            >
              <input
                ref={pathInputRef}
                className="text-field"
                name="akmux-directory-path"
                autoComplete="off"
                spellCheck={false}
                value={path}
                onChange={event => setPath(event.target.value)}
              />
            </form>
          </div>
          <div className="mt-3 h-[min(390px,48vh)] overflow-y-auto rounded-lg border border-border bg-surface p-1">
            {loading ? (
              <div className="p-4 text-sm text-muted-foreground">{props.t('loading')}</div>
            ) : error ? (
              <div className="p-4 text-sm text-destructive">{error}</div>
            ) : listing?.entries.length ? (
              listing.entries.map(entry => (
                <button key={entry.path} className="flex h-10 w-full items-center gap-2 rounded-md px-3 text-left text-sm hover:bg-accent" onClick={() => void load(entry.path)}>
                  <Folder className="size-4 shrink-0 text-primary" />
                  <span className="min-w-0 flex-1 truncate">{entry.name}</span>
                  <ChevronRight className="size-4 text-muted-foreground" />
                </button>
              ))
            ) : (
              <div className="p-4 text-sm text-muted-foreground">{props.t('noHistory')}</div>
            )}
          </div>
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={() => props.onOpenChange(false)}>
            {props.t('cancel')}
          </Button>
          <Button disabled={!listing} onClick={() => listing && props.onChoose(listing.path)}>
            {props.t('useDirectory')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
