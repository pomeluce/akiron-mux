import { Button } from '@/shared/ui/button';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog';

export function ConfirmDialog({ open, title, body, confirmLabel, cancelLabel, destructive, onOpenChange, onConfirm }: { open: boolean; title: string; body: string; confirmLabel: string; cancelLabel: string; destructive?: boolean; onOpenChange: (open: boolean) => void; onConfirm: () => void }) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="w-[min(430px,calc(100vw-28px))]">
        <DialogHeader><DialogTitle>{title}</DialogTitle><DialogDescription>{body}</DialogDescription></DialogHeader>
        <DialogFooter><Button variant="ghost" onClick={() => onOpenChange(false)}>{cancelLabel}</Button><Button variant={destructive ? 'destructive' : 'default'} onClick={() => { onConfirm(); onOpenChange(false); }}>{confirmLabel}</Button></DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
