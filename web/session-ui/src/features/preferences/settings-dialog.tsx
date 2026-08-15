import { Folder } from 'lucide-react';
import { useEffect, useState } from 'react';
import { Button } from '@/shared/ui/button';
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog';
import type { ClientPreferences, Locale, ThemeMode } from '@/types';
import type { MessageKey } from '@/shared/lib/i18n';
import { DirectoryDialog } from '@/features/workspaces/directory-dialog';

interface SettingsDialogProps {
  open: boolean;
  preferences: ClientPreferences;
  generalRoot: string;
  t: (key: MessageKey) => string;
  onOpenChange: (open: boolean) => void;
  onSave: (preferences: ClientPreferences, generalRoot: string) => Promise<void>;
}

export function SettingsDialog(props: SettingsDialogProps) {
  const [draft, setDraft] = useState(props.preferences);
  const [root, setRoot] = useState(props.generalRoot);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!props.open) return;
    setDraft(props.preferences);
    setRoot(props.generalRoot);
    setError(null);
  }, [props.open, props.preferences, props.generalRoot]);

  const submit = async () => {
    setSaving(true);
    try {
      await props.onSave(draft, root);
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
        <DialogContent className="w-[min(620px,calc(100vw-28px))]">
          <DialogHeader>
            <DialogTitle>{props.t('settingsTitle')}</DialogTitle>
          </DialogHeader>
          <div className="max-h-[68vh] space-y-6 overflow-y-auto p-6">
            <SettingsSection title={props.t('appearance')}>
              <div>
                <span className="field-label">{props.t('theme')}</span>
                <div className="segmented-control">
                  {(['system', 'light', 'dark'] as ThemeMode[]).map(theme => (
                    <button key={theme} data-active={draft.theme === theme} onClick={() => setDraft(value => ({ ...value, theme }))}>
                      {props.t(theme)}
                    </button>
                  ))}
                </div>
              </div>
              <label className="flex items-center justify-between gap-4 text-sm">
                <span>{props.t('acrylic')}</span>
                <input
                  type="checkbox"
                  className="size-4 accent-primary"
                  checked={draft.acrylic}
                  onChange={event => setDraft(value => ({ ...value, acrylic: event.target.checked }))}
                />
              </label>
              <div>
                <div className="mb-2 flex justify-between text-sm">
                  <span>{props.t('acrylicStrength')}</span>
                  <span className="text-muted-foreground">{draft.acrylicStrength}%</span>
                </div>
                <input
                  className="w-full accent-primary"
                  type="range"
                  min="20"
                  max="90"
                  step="5"
                  value={draft.acrylicStrength}
                  disabled={!draft.acrylic}
                  onChange={event => setDraft(value => ({ ...value, acrylicStrength: Number(event.target.value) }))}
                />
              </div>
            </SettingsSection>
            <SettingsSection title={props.t('language')}>
              <div className="grid grid-cols-2 gap-2">
                {(
                  [
                    ['en', 'English'],
                    ['zh-CN', '中文'],
                  ] as Array<[Locale, string]>
                ).map(([locale, label]) => (
                  <Button
                    key={locale}
                    className={locale === 'zh-CN' ? 'font-zh' : ''}
                    variant={draft.locale === locale ? 'secondary' : 'outline'}
                    onClick={() => setDraft(value => ({ ...value, locale }))}
                  >
                    {label}
                  </Button>
                ))}
              </div>
            </SettingsSection>
            <SettingsSection title={props.t('backendAddress')}>
              <input
                className="text-field"
                value={draft.backendAddress}
                onChange={event => setDraft(value => ({ ...value, backendAddress: event.target.value }))}
                placeholder="http://127.0.0.1:17321"
              />
              <p className="m-0 text-xs text-muted-foreground">{props.t('backendHint')}</p>
            </SettingsSection>
            <SettingsSection title={props.t('generalRoot')}>
              <div className="flex gap-2">
                <div className="flex h-10 min-w-0 flex-1 items-center gap-2 rounded-lg border border-border px-3 text-sm">
                  <Folder className="size-4 shrink-0 text-primary" />
                  <span className="truncate">{root}</span>
                </div>
                <Button variant="outline" onClick={() => setPickerOpen(true)}>
                  {props.t('browse')}
                </Button>
              </div>
            </SettingsSection>
            {error && <div className="text-sm text-destructive">{error}</div>}
          </div>
          <DialogFooter>
            <Button variant="ghost" onClick={() => props.onOpenChange(false)}>
              {props.t('cancel')}
            </Button>
            <Button disabled={saving || !root} onClick={() => void submit()}>
              {props.t('save')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <DirectoryDialog
        open={pickerOpen}
        backendAddress={draft.backendAddress}
        initialPath={root}
        t={props.t}
        onOpenChange={setPickerOpen}
        onChoose={value => {
          setRoot(value);
          setPickerOpen(false);
        }}
      />
    </>
  );
}

function SettingsSection({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="space-y-4">
      <h3 className="m-0 text-xs font-semibold uppercase text-muted-foreground">{title}</h3>
      {children}
    </section>
  );
}
