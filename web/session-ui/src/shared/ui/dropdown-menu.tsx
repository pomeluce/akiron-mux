import * as DropdownMenuPrimitive from '@radix-ui/react-dropdown-menu';
import type * as React from 'react';
import { cn } from '@/shared/lib/utils';

export const DropdownMenu = DropdownMenuPrimitive.Root;
export const DropdownMenuTrigger = DropdownMenuPrimitive.Trigger;

export function DropdownMenuContent({ className, ...props }: React.ComponentProps<typeof DropdownMenuPrimitive.Content>) {
  return (
    <DropdownMenuPrimitive.Portal>
      <DropdownMenuPrimitive.Content
        sideOffset={6}
        className={cn('floating-acrylic z-50 min-w-48 rounded-lg border border-border p-1 text-popover-foreground shadow-menu outline-none', className)}
        {...props}
      />
    </DropdownMenuPrimitive.Portal>
  );
}

export function DropdownMenuItem({ className, ...props }: React.ComponentProps<typeof DropdownMenuPrimitive.Item>) {
  return (
    <DropdownMenuPrimitive.Item
      className={cn('flex h-8 cursor-default select-none items-center gap-2 rounded-md px-2 text-sm outline-none focus:bg-accent data-[disabled]:opacity-40 [&_svg]:size-4', className)}
      {...props}
    />
  );
}

export const DropdownMenuSeparator = (props: React.ComponentProps<typeof DropdownMenuPrimitive.Separator>) => (
  <DropdownMenuPrimitive.Separator className="my-1 h-px bg-border" {...props} />
);
