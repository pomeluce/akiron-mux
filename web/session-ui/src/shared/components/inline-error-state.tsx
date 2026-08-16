import { CircleAlert, RefreshCw } from 'lucide-react';
import { cn } from '@/shared/lib/utils';
import { Button } from '@/shared/ui/button';

export function InlineErrorState({
  title,
  message,
  retryLabel,
  onRetry,
  compact = false,
}: {
  title: string;
  message: string;
  retryLabel?: string;
  onRetry?: () => void;
  compact?: boolean;
}) {
  return (
    <div className={cn('flex', compact ? 'items-start gap-3 rounded-lg bg-destructive/8 p-3 text-left' : 'h-full min-h-44 flex-col items-center justify-center px-6 py-8 text-center')}>
      <span className={cn('grid shrink-0 place-items-center rounded-full bg-destructive/10 text-destructive', compact ? 'size-8' : 'mb-3 size-10')}>
        <CircleAlert className={compact ? 'size-4' : 'size-5'} />
      </span>
      <div className={compact ? 'min-w-0 flex-1' : undefined}>
        <strong className="text-sm font-medium text-foreground">{title}</strong>
        <p className={cn('mb-0 max-w-sm text-xs leading-5 text-muted-foreground', compact ? 'mt-0.5' : 'mt-1.5')}>{message}</p>
        {onRetry && retryLabel && (
          <Button variant="outline" size="sm" className={compact ? 'mt-2.5' : 'mt-4'} onClick={onRetry}>
            <RefreshCw />
            {retryLabel}
          </Button>
        )}
      </div>
    </div>
  );
}
