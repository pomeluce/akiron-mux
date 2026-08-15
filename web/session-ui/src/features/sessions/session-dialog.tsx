import { Check, ChevronDown, Folder } from 'lucide-react';
import { useEffect, useState } from 'react';
import { AgentIcon } from '@/shared/components/agent-icon';
import { sessionApi } from '@/shared/lib/api';
import { basename } from '@/shared/lib/utils';
import { Button } from '@/shared/ui/button';
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/shared/ui/dropdown-menu';
import type { Agent } from '@/types';
import type { MessageKey } from '@/shared/lib/i18n';
import { DirectoryDialog } from '@/features/workspaces/directory-dialog';

interface SessionDialogProps {
  open: boolean;
  mode: 'general' | 'project';
  backendAddress: string;
  initialDirectory: string;
  t: (key: MessageKey) => string;
  onOpenChange: (open: boolean) => void;
  onCreate: (agent: Agent, cwd: string) => Promise<void>;
}

export function SessionDialog(props: SessionDialogProps) {
  const [agent, setAgent] = useState<Agent>('codex');
  const [directory, setDirectory] = useState(props.initialDirectory);
  const [isolate, setIsolate] = useState(false);
  const [subdirectory, setSubdirectory] = useState('');
  const [pickerOpen, setPickerOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!props.open) return;
    setAgent('codex');
    setDirectory(props.initialDirectory);
    setIsolate(false);
    setSubdirectory('');
    setError(null);
  }, [props.open, props.initialDirectory]);

  const submit = async () => {
    setSaving(true);
    setError(null);
    try {
      let cwd = directory;
      if (props.mode === 'general' && isolate) {
        const name = subdirectory.trim();
        if (!name || name.includes('/') || name.includes('\\')) throw new Error('Enter a valid subdirectory name');
        await sessionApi.createDirectory(props.backendAddress, directory, name);
        cwd = `${directory.replace(/\/$/, '')}/${name}`;
      }
      await props.onCreate(agent, cwd);
      props.onOpenChange(false);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setSaving(false);
    }
  };

  return (
    <>
      <Dialog open={props.open} onOpenChange={props.onOpenChange}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{props.t(props.mode === 'project' ? 'projectSession' : 'newGeneral')}</DialogTitle>
          </DialogHeader>
          <div className="space-y-5 p-6">
            <div>
              <span className="field-label">{props.t('agent')}</span>
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button variant="outline" className="h-11 w-full justify-start px-3">
                    <AgentIcon agent={agent} className="size-6" />
                    {agent === 'codex' ? 'Codex' : 'Claude Code'}
                    <ChevronDown className="ml-auto" />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent className="w-[var(--radix-dropdown-menu-trigger-width)]">
                  {(['codex', 'claude'] as const).map(value => (
                    <DropdownMenuItem key={value} onSelect={() => setAgent(value)}>
                      <AgentIcon agent={value} className="size-6" />
                      {value === 'codex' ? 'Codex' : 'Claude Code'}
                      {agent === value && <Check className="ml-auto" />}
                    </DropdownMenuItem>
                  ))}
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
            <div>
              <span className="field-label">{props.t('directory')}</span>
              <div className="flex gap-2">
                <div className="flex h-10 min-w-0 flex-1 items-center gap-2 rounded-lg border border-border bg-surface px-3 text-sm" title={directory}>
                  <Folder className="size-4 shrink-0 text-primary" />
                  <span className="truncate">{directory || basename(directory)}</span>
                </div>
                {props.mode === 'project' && (
                  <Button variant="outline" onClick={() => setPickerOpen(true)}>
                    {props.t('browse')}
                  </Button>
                )}
              </div>
            </div>
            {props.mode === 'general' && (
              <div className="rounded-lg bg-muted/65 p-3">
                <label className="flex items-center gap-2 text-sm">
                  <input type="checkbox" checked={isolate} onChange={event => setIsolate(event.target.checked)} className="accent-primary" />
                  {props.t('isolate')}
                </label>
                {isolate && (
                  <div className="mt-3">
                    <span className="field-label">{props.t('subdirectory')}</span>
                    <input className="text-field" value={subdirectory} onChange={event => setSubdirectory(event.target.value)} placeholder="session-workspace" />
                  </div>
                )}
              </div>
            )}
            {error && <div className="text-sm text-destructive">{error}</div>}
          </div>
          <DialogFooter>
            <Button variant="ghost" onClick={() => props.onOpenChange(false)}>
              {props.t('cancel')}
            </Button>
            <Button disabled={saving || !directory} onClick={() => void submit()}>
              {props.t('create')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <DirectoryDialog
        open={pickerOpen}
        backendAddress={props.backendAddress}
        initialPath={directory}
        t={props.t}
        onOpenChange={setPickerOpen}
        onChoose={path => {
          setDirectory(path);
          setPickerOpen(false);
        }}
      />
    </>
  );
}
