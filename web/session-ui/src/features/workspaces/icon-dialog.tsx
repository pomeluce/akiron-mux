import { useEffect, useState } from 'react';
import { IconPicker, type WorkspaceIconName } from '@/shared/components/workspace-icon';
import { Button } from '@/shared/ui/button';
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog';
import type { MessageKey } from '@/shared/lib/i18n';

export function IconDialog({
  open,
  title,
  value,
  t,
  onOpenChange,
  onSave,
}: {
  open: boolean;
  title: string;
  value: WorkspaceIconName;
  t: (key: MessageKey) => string;
  onOpenChange: (open: boolean) => void;
  onSave: (value: WorkspaceIconName) => void;
}) {
  const [draft, setDraft] = useState(value);
  useEffect(() => {
    if (open) setDraft(value);
  }, [open, value]);
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="w-[min(430px,calc(100vw-28px))]">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
        </DialogHeader>
        <div className="p-6">
          <IconPicker value={draft} onChange={setDraft} />
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            {t('cancel')}
          </Button>
          <Button
            onClick={() => {
              onSave(draft);
              onOpenChange(false);
            }}
          >
            {t('save')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
