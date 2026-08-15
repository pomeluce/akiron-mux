import {
  Archive,
  Bot,
  BrainCircuit,
  BookOpen,
  Boxes,
  BriefcaseBusiness,
  Building2,
  CalendarDays,
  ChartNoAxesCombined,
  Cloud,
  Code2,
  Cog,
  Cpu,
  Database,
  FileCode2,
  FlaskConical,
  Folder,
  Gamepad2,
  GitBranch,
  Globe2,
  Hammer,
  HardDrive,
  House,
  KeyRound,
  Layers3,
  Library,
  Monitor,
  Network,
  Package,
  Palette,
  Rocket,
  ShieldCheck,
  Sparkles,
  SquareTerminal,
  Wrench,
  type LucideIcon,
} from 'lucide-react';
import { cn } from '@/shared/lib/utils';
import { Tooltip } from '@/shared/ui/tooltip';

export const workspaceIconOptions = {
  folder: { icon: Folder, label: 'Folder' },
  code: { icon: Code2, label: 'Code' },
  terminal: { icon: SquareTerminal, label: 'Terminal' },
  boxes: { icon: Boxes, label: 'Modules' },
  wrench: { icon: Wrench, label: 'Tools' },
  book: { icon: BookOpen, label: 'Docs' },
  database: { icon: Database, label: 'Database' },
  globe: { icon: Globe2, label: 'Web' },
  cpu: { icon: Cpu, label: 'System' },
  package: { icon: Package, label: 'Package' },
  layers: { icon: Layers3, label: 'Layers' },
  work: { icon: BriefcaseBusiness, label: 'Work' },
  lab: { icon: FlaskConical, label: 'Lab' },
  sparkles: { icon: Sparkles, label: 'Creative' },
  archive: { icon: Archive, label: 'Archive' },
  bot: { icon: Bot, label: 'Agent' },
  brain: { icon: BrainCircuit, label: 'AI' },
  building: { icon: Building2, label: 'Organization' },
  calendar: { icon: CalendarDays, label: 'Planning' },
  chart: { icon: ChartNoAxesCombined, label: 'Analytics' },
  cloud: { icon: Cloud, label: 'Cloud' },
  settings: { icon: Cog, label: 'Configuration' },
  fileCode: { icon: FileCode2, label: 'Source files' },
  game: { icon: Gamepad2, label: 'Game' },
  git: { icon: GitBranch, label: 'Git' },
  hammer: { icon: Hammer, label: 'Build' },
  drive: { icon: HardDrive, label: 'Storage' },
  home: { icon: House, label: 'Home' },
  key: { icon: KeyRound, label: 'Security' },
  library: { icon: Library, label: 'Library' },
  monitor: { icon: Monitor, label: 'Desktop' },
  network: { icon: Network, label: 'Network' },
  palette: { icon: Palette, label: 'Design' },
  rocket: { icon: Rocket, label: 'Launch' },
  shield: { icon: ShieldCheck, label: 'Protected' },
} satisfies Record<string, { icon: LucideIcon; label: string }>;

export type WorkspaceIconName = keyof typeof workspaceIconOptions;

export function WorkspaceIcon({ name = 'folder', className }: { name?: WorkspaceIconName; className?: string }) {
  const Icon = workspaceIconOptions[name]?.icon || Folder;
  return <Icon className={cn('size-4 shrink-0', className)} />;
}

export function IconPicker({ value, onChange }: { value: WorkspaceIconName; onChange: (value: WorkspaceIconName) => void }) {
  return (
    <div className="grid grid-cols-7 gap-2 max-[520px]:grid-cols-5">
      {(Object.entries(workspaceIconOptions) as Array<[WorkspaceIconName, { icon: LucideIcon; label: string }]>).map(([name, option]) => {
        const Icon = option.icon;
        return (
          <Tooltip key={name} label={option.label}>
            <button
              type="button"
              className={cn('grid size-10 place-items-center rounded-lg border border-border text-muted-foreground hover:bg-accent hover:text-foreground', value === name && 'border-foreground/35 bg-accent text-foreground')}
              aria-pressed={value === name}
              onClick={() => onChange(name)}
            >
              <Icon className="size-4" />
            </button>
          </Tooltip>
        );
      })}
    </div>
  );
}
