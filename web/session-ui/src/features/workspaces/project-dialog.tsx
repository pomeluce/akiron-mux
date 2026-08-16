import { Folder } from 'lucide-react';
import { useEffect, useState } from 'react';
import { InlineErrorState } from '@/shared/components/inline-error-state';
import { isServiceUnavailable } from '@/shared/lib/api';
import { Button } from '@/shared/ui/button';
import { IconPicker, type WorkspaceIconName } from '@/shared/components/workspace-icon';
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog';
import type { Project } from '@/types';
import type { MessageKey } from '@/shared/lib/i18n';
import { DirectoryDialog } from './directory-dialog';

interface ProjectDialogProps {
  open: boolean;
  project: Project | null;
  icon: WorkspaceIconName;
  backendAddress: string;
  initialPath: string;
  t: (key: MessageKey) => string;
  onOpenChange: (open: boolean) => void;
  onSave: (path: string, name: string, icon: WorkspaceIconName) => Promise<void>;
}

export function ProjectDialog(props: ProjectDialogProps) {
  const [path, setPath] = useState('');
  const [name, setName] = useState('');
  const [pickerOpen, setPickerOpen] = useState(false);
  const [icon, setIcon] = useState<WorkspaceIconName>(props.icon);
  const [error, setError] = useState<'backend' | 'save' | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!props.open) return;
    setPath(props.project?.path || '');
    setName(props.project?.name || '');
    setIcon(props.icon);
    setError(null);
  }, [props.open, props.project, props.icon]);

  const submit = async () => {
    setSaving(true);
    try {
      await props.onSave(path, name, icon);
      props.onOpenChange(false);
    } catch (cause) {
      setError(isServiceUnavailable(cause) ? 'backend' : 'save');
    } finally {
      setSaving(false);
    }
  };

  return (
    <>
      <Dialog open={props.open} onOpenChange={props.onOpenChange}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{props.t(props.project ? 'editProject' : 'addProject')}</DialogTitle>
          </DialogHeader>
          <div className="space-y-5 p-6">
            <div>
              <span className="field-label">{props.t('directory')}</span>
              <div className="flex gap-2">
                <div className="flex h-10 min-w-0 flex-1 items-center gap-2 rounded-lg border border-border px-3 text-sm">
                  <Folder className="size-4 shrink-0 text-primary" />
                  <span className="truncate">{path}</span>
                </div>
                <Button variant="outline" onClick={() => setPickerOpen(true)}>
                  {props.t('browse')}
                </Button>
              </div>
            </div>
            <div>
              <label className="field-label" htmlFor="project-name">
                {props.t('projectName')}
              </label>
              <input
                id="project-name"
                className="text-field"
                name="akmux-project-name"
                autoComplete="off"
                spellCheck={false}
                value={name}
                onChange={event => setName(event.target.value)}
              />
            </div>
            <div>
              <span className="field-label">{props.t('icon')}</span>
              <IconPicker value={icon} onChange={setIcon} />
            </div>
            {error && (
              <InlineErrorState
                compact
                title={props.t(error === 'backend' ? 'backendUnavailable' : 'projectSaveFailed')}
                message={props.t(error === 'backend' ? 'backendUnavailableHint' : 'projectSaveFailedHint')}
              />
            )}
          </div>
          <DialogFooter>
            <Button variant="ghost" onClick={() => props.onOpenChange(false)}>
              {props.t('cancel')}
            </Button>
            <Button disabled={saving || !path} onClick={() => void submit()}>
              {props.t('save')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <DirectoryDialog
        open={pickerOpen}
        backendAddress={props.backendAddress}
        initialPath=""
        t={props.t}
        onOpenChange={setPickerOpen}
        onChoose={value => {
          setPath(value);
          setPickerOpen(false);
        }}
      />
    </>
  );
}
