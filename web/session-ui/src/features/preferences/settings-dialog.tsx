import { Folder, Languages, MonitorCog, Palette } from 'lucide-react';
import { useEffect, useState } from 'react';
import { InlineErrorState } from '@/shared/components/inline-error-state';
import { isServiceUnavailable } from '@/shared/lib/api';
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
  const [error, setError] = useState<'backend' | 'save' | null>(null);
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
      setError(isServiceUnavailable(cause) ? 'backend' : 'save');
    } finally {
      setSaving(false);
    }
  };

  return (
    <>
      <Dialog open={props.open} onOpenChange={props.onOpenChange}>
        <DialogContent className="settings-dialog w-[min(700px,calc(100vw-28px))]">
          <DialogHeader>
            <DialogTitle>{props.t('settingsTitle')}</DialogTitle>
          </DialogHeader>
          <div className="settings-content max-h-[70vh] overflow-y-auto px-6 py-2">
            <SettingsSection icon={<Palette />} title={props.t('appearance')}>
              <SettingsRow label={props.t('theme')} baseline>
                <div className="segmented-control segmented-control-theme">
                  {(['system', 'light', 'dark'] as ThemeMode[]).map(theme => (
                    <button key={theme} data-active={draft.theme === theme} onClick={() => setDraft(value => ({ ...value, theme }))}>
                      {props.t(theme)}
                    </button>
                  ))}
                </div>
              </SettingsRow>
              <SettingsRow label={props.t('acrylic')}>
                <label className="settings-switch">
                  <input type="checkbox" className="sr-only" checked={draft.acrylic} onChange={event => setDraft(value => ({ ...value, acrylic: event.target.checked }))} />
                  <span aria-hidden="true" />
                </label>
              </SettingsRow>
              <SettingsRow label={props.t('acrylicStrength')} value={`${draft.acrylicStrength}%`} stacked>
                <input
                  className="w-full accent-primary"
                  type="range"
                  aria-label={props.t('acrylicStrength')}
                  min="0"
                  max="100"
                  step="5"
                  value={draft.acrylicStrength}
                  disabled={!draft.acrylic}
                  onChange={event => setDraft(value => ({ ...value, acrylicStrength: Number(event.target.value) }))}
                />
              </SettingsRow>
              <SettingsRow label={props.t('terminalFontSize')} value={`${draft.terminalFontSize}px`} stacked>
                <input
                  className="w-full accent-primary"
                  type="range"
                  aria-label={props.t('terminalFontSize')}
                  min="10"
                  max="24"
                  step="1"
                  value={draft.terminalFontSize}
                  onChange={event => setDraft(value => ({ ...value, terminalFontSize: Number(event.target.value) }))}
                />
              </SettingsRow>
            </SettingsSection>
            <SettingsSection icon={<Languages />} title={props.t('language')}>
              <SettingsRow label={props.t('language')} baseline>
                <div className="segmented-control segmented-control-language">
                  {(
                    [
                      ['en', 'English'],
                      ['zh-CN', '中文'],
                    ] as Array<[Locale, string]>
                  ).map(([locale, label]) => (
                    <button
                      key={locale}
                      className={locale === 'zh-CN' ? 'font-zh' : ''}
                      data-active={draft.locale === locale}
                      onClick={() => setDraft(value => ({ ...value, locale }))}
                    >
                      {label}
                    </button>
                  ))}
                </div>
              </SettingsRow>
            </SettingsSection>
            <SettingsSection icon={<MonitorCog />} title={props.t('workspaceSettings')}>
              <SettingsRow label={props.t('backendAddress')} hint={props.t('backendHint')} stacked>
                <input
                  className="text-field"
                  name="akmux-backend-address"
                  autoComplete="off"
                  spellCheck={false}
                  value={draft.backendAddress}
                  onChange={event => setDraft(value => ({ ...value, backendAddress: event.target.value }))}
                />
              </SettingsRow>
              <SettingsRow label={props.t('generalRoot')} stacked>
                <div className="flex gap-2">
                  <div className="flex h-10 min-w-0 flex-1 items-center gap-2 rounded-lg border border-border px-3 text-sm">
                    <Folder className="size-4 shrink-0 text-primary" />
                    <span className="truncate">{root}</span>
                  </div>
                  <Button variant="outline" onClick={() => setPickerOpen(true)}>
                    {props.t('browse')}
                  </Button>
                </div>
              </SettingsRow>
            </SettingsSection>
            {error && (
              <InlineErrorState
                compact
                title={props.t(error === 'backend' ? 'backendUnavailable' : 'settingsSaveFailed')}
                message={props.t(error === 'backend' ? 'backendUnavailableHint' : 'settingsSaveFailedHint')}
              />
            )}
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
        initialPath=""
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

function SettingsSection({ icon, title, children }: { icon: React.ReactNode; title: string; children: React.ReactNode }) {
  return (
    <section className="settings-section">
      <h3>
        {icon}
        {title}
      </h3>
      <div className="settings-section-body">{children}</div>
    </section>
  );
}

function SettingsRow({
  label,
  value,
  hint,
  stacked = false,
  baseline = false,
  children,
}: {
  label: string;
  value?: string;
  hint?: string;
  stacked?: boolean;
  baseline?: boolean;
  children: React.ReactNode;
}) {
  return (
    <div className={stacked ? 'settings-row settings-row-stacked' : baseline ? 'settings-row settings-row-baseline' : 'settings-row'}>
      <div className="min-w-0">
        <div className="flex items-center gap-2 text-sm font-medium">
          <span>{label}</span>
          {value && <span className="text-xs font-normal text-muted-foreground">{value}</span>}
        </div>
        {hint && <p className="m-0 mt-1 text-xs text-muted-foreground">{hint}</p>}
      </div>
      <div className={stacked ? 'w-full' : 'shrink-0'}>{children}</div>
    </div>
  );
}
