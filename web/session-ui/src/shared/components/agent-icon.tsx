import { cn } from '@/shared/lib/utils';
import type { Agent } from '@/types';

export function AgentIcon({ agent, className }: { agent: Agent; className?: string }) {
  return (
    <span
      className={cn(
        'grid size-7 shrink-0 place-items-center rounded-md',
        agent === 'codex' ? 'bg-emerald-500/12' : 'bg-orange-500/14',
        className,
      )}
    >
      <img className="size-4" src={`/${agent === 'codex' ? 'openai' : 'claude'}.svg`} alt="" />
    </span>
  );
}
