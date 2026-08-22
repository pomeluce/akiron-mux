import { ArrowDown, ArrowUp, Folder, Languages, MonitorCog, Palette, Plus, Power, Server, Trash2 } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { InlineErrorState } from '@/shared/components/inline-error-state';
import { isServiceUnavailable } from '@/shared/lib/api';
import { Button } from '@/shared/ui/button';
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog';
import type { BackendLifecycleOutcome, BackendProfile, ClientPreferences, Locale, ThemeMode } from '@/types';
import type { BackendManager } from '@/features/backends/use-backends';
import { desktopShell } from '@/features/desktop/desktop-shell';
import type { MessageKey } from '@/shared/lib/i18n';
import { DirectoryDialog } from '@/features/workspaces/directory-dialog';

interface SettingsDialogProps {
  open: boolean;
  preferences: ClientPreferences;
  backends: BackendManager;
  workspaceEnabled: boolean;
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
  const [backendDraft, setBackendDraft] = useState<BackendProfile>(props.backends.active);
  const backendPairingRef = useRef<HTMLInputElement>(null);
  const [backendMessage, setBackendMessage] = useState('');
  const [pendingIdentity, setPendingIdentity] = useState<{ challengeId: string; observedInstanceId: string } | null>(null);

  const discardPendingIdentity = () => {
    if (pendingIdentity) void props.backends.cancelIdentity(pendingIdentity.challengeId);
    setPendingIdentity(null);
  };

  const applyBackendOutcome = (outcome: BackendLifecycleOutcome) => {
    if (outcome.type === 'identityConfirmationRequired') {
      setPendingIdentity({ challengeId: outcome.challengeId, observedInstanceId: outcome.observedInstanceId });
      if (backendPairingRef.current) backendPairingRef.current.value = '';
      setBackendMessage(props.t('identityChanged'));
      return;
    }
    const saved = outcome.state.profiles.find(profile => profile.id === backendDraft.id);
    if (saved) setBackendDraft(saved);
    setPendingIdentity(null);
    if (backendPairingRef.current) backendPairingRef.current.value = '';
    if (outcome.type === 'authenticationRequired') setBackendMessage(props.t('backendReauthRequired'));
    else if (outcome.type === 'offline') setBackendMessage(props.t('backendUnavailableHint'));
    else if (outcome.type === 'appliedWithWarning') setBackendMessage(outcome.warning);
    else setBackendMessage(props.t('connectionPassed'));
  };

  useEffect(() => {
    if (!props.open) return;
    setDraft(props.preferences);
    setRoot(props.generalRoot);
    setError(null);
    setBackendDraft(props.backends.active);
    if (backendPairingRef.current) backendPairingRef.current.value = '';
    setBackendMessage('');
    setPendingIdentity(null);
  }, [props.open]);

  const testBackend = async () => {
    try {
      const health = await props.backends.test(backendDraft);
      setBackendMessage(`${props.t('connectionPassed')} · ${health.apiProtocol}`);
    } catch {
      setBackendMessage(props.t('backendUnavailableHint'));
    }
  };

  const saveBackend = async () => {
    const pairingLink = backendPairingRef.current?.value.trim() || '';
    try {
      const outcome = pendingIdentity
        ? await props.backends.confirmIdentity(pendingIdentity.challengeId)
        : await props.backends.save(backendDraft, pairingLink);
      applyBackendOutcome(outcome);
    } catch {
      setBackendMessage(props.t('backendUnavailableHint'));
    }
  };

  const newBackend = () => {
    discardPendingIdentity();
    setBackendDraft({
      id: crypto.randomUUID(),
      name: props.t('remoteBackend'),
      kind: 'remote',
      address: 'https://',
      instanceId: null,
      hasCredential: false,
      requiresAuth: true,
      capabilities: [],
    });
    if (backendPairingRef.current) backendPairingRef.current.value = '';
    setBackendMessage('');
  };

  const setOpen = (open: boolean) => {
    if (!open) discardPendingIdentity();
    props.onOpenChange(open);
  };

  const submit = async () => {
    setSaving(true);
    try {
      await props.onSave(draft, root);
      setOpen(false);
    } catch (cause) {
      setError(isServiceUnavailable(cause) ? 'backend' : 'save');
    } finally {
      setSaving(false);
    }
  };

  return (
    <>
      <Dialog open={props.open} onOpenChange={setOpen}>
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
            {desktopShell && (
              <SettingsSection icon={<Power />} title={props.t('application')}>
                <SettingsRow label={props.t('closeBehavior')} baseline>
                  <div className="segmented-control segmented-control-close">
                    {(['tray', 'quit'] as const).map(behavior => (
                      <button
                        key={behavior}
                        data-active={draft.closeBehavior === behavior}
                        onClick={() => setDraft(value => ({ ...value, closeBehavior: behavior }))}
                      >
                        {props.t(behavior === 'tray' ? 'closeToTray' : 'quitOnClose')}
                      </button>
                    ))}
                  </div>
                </SettingsRow>
              </SettingsSection>
            )}
            <SettingsSection icon={<MonitorCog />} title={props.t('workspaceSettings')}>
              {desktopShell ? (
                <>
              {props.backends.state.profiles.length > 0 && (
                <SettingsRow label={props.t('backends')} stacked>
                  <div className="flex flex-wrap gap-2">
                    {props.backends.state.profiles.map(profile => (
                      <Button
                        key={profile.id}
                        size="sm"
                        variant={profile.id === backendDraft.id ? 'secondary' : 'outline'}
                        onClick={() => {
                          discardPendingIdentity();
                          setBackendDraft(profile);
                          if (backendPairingRef.current) backendPairingRef.current.value = '';
                          setBackendMessage('');
                        }}
                      >
                        <Server className="size-3.5" /> {profile.kind === 'local' ? props.t('localBackend') : profile.name}
                      </Button>
                    ))}
                    <Button size="sm" variant="outline" onClick={newBackend}>
                      <Plus className="size-3.5" /> {props.t('addBackend')}
                    </Button>
                    <Button
                      size="icon-sm"
                      variant="ghost"
                      aria-label="Move backend up"
                      onClick={() => {
                        const ids = props.backends.state.profiles.map(profile => profile.id);
                        const index = ids.indexOf(backendDraft.id);
                        if (index > 0) [ids[index - 1], ids[index]] = [ids[index], ids[index - 1]];
                        void props.backends.reorder(ids);
                      }}
                    >
                      <ArrowUp />
                    </Button>
                    <Button
                      size="icon-sm"
                      variant="ghost"
                      aria-label="Move backend down"
                      onClick={() => {
                        const ids = props.backends.state.profiles.map(profile => profile.id);
                        const index = ids.indexOf(backendDraft.id);
                        if (index >= 0 && index < ids.length - 1) [ids[index], ids[index + 1]] = [ids[index + 1], ids[index]];
                        void props.backends.reorder(ids);
                      }}
                    >
                      <ArrowDown />
                    </Button>
                  </div>
                </SettingsRow>
              )}
              <SettingsRow label={props.t('backendName')} stacked>
                <input
                  className="text-field"
                  value={backendDraft.kind === 'local' ? props.t('localBackend') : backendDraft.name}
                  disabled={backendDraft.id === 'local'}
                  onChange={event => setBackendDraft(value => ({ ...value, name: event.target.value }))}
                />
              </SettingsRow>
              <SettingsRow label={props.t('backends')} baseline>
                <div className="segmented-control segmented-control-language">
                  {(['local', 'remote'] as const).map(kind => (
                    <button key={kind} disabled data-active={backendDraft.kind === kind}>
                      {props.t(kind === 'local' ? 'localBackend' : 'remoteBackend')}
                    </button>
                  ))}
                </div>
              </SettingsRow>
              <SettingsRow label={props.t('backendAddress')} hint={props.t('backendHint')} stacked>
                <input
                  className="text-field"
                  name="akmux-backend-address"
                  autoComplete="off"
                  spellCheck={false}
                  value={backendDraft.address}
                  onChange={event => setBackendDraft(value => ({ ...value, address: event.target.value }))}
                />
              </SettingsRow>
              {backendDraft.kind === 'remote' && (
                <SettingsRow label={props.t('backendPairingLink')} hint={props.t('backendPairingHint')} stacked>
                  <input
                    className="text-field"
                    ref={backendPairingRef}
                    autoComplete="off"
                    spellCheck={false}
                  />
                  {backendDraft.requiresAuth && <span className="text-xs text-destructive">{props.t('backendReauthRequired')}</span>}
                </SettingsRow>
              )}
              <SettingsRow label={props.t('testConnection')} stacked>
                <div className="flex items-center gap-2">
                  <Button variant="outline" onClick={() => void testBackend()}>{props.t('testConnection')}</Button>
                  <Button onClick={() => void saveBackend()}>{pendingIdentity ? props.t('confirm') : props.t('save')}</Button>
                  {backendDraft.id !== 'local' && props.backends.state.profiles.some(profile => profile.id === backendDraft.id) && (
                    <Button
                      variant="destructive"
                      size="icon"
                      aria-label={props.t('remove')}
                      onClick={() => void props.backends.remove(backendDraft.id).then(outcome => {
                        setBackendDraft(outcome.state.profiles.find(profile => profile.id === outcome.state.activeProfileId) || outcome.state.profiles[0]);
                        if (outcome.type === 'appliedWithWarning') setBackendMessage(outcome.warning);
                      })}
                    >
                      <Trash2 />
                    </Button>
                  )}
                  {backendMessage && <span className="text-xs text-muted-foreground">{backendMessage}</span>}
                </div>
              </SettingsRow>
                </>
              ) : (
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
              )}
              {props.workspaceEnabled ? <SettingsRow label={props.t('generalRoot')} stacked>
                <div className="flex gap-2">
                  <div className="flex h-10 min-w-0 flex-1 items-center gap-2 rounded-lg border border-border px-3 text-sm">
                    <Folder className="size-4 shrink-0 text-primary" />
                    <span className="truncate">{root}</span>
                  </div>
                  <Button variant="outline" onClick={() => setPickerOpen(true)}>
                    {props.t('browse')}
                  </Button>
                </div>
              </SettingsRow> : <p className="px-1 text-xs text-muted-foreground">{props.t('workspaceCapabilityUnavailable')}</p>}
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
            <Button variant="ghost" onClick={() => setOpen(false)}>
              {props.t('cancel')}
            </Button>
            <Button disabled={saving || (props.workspaceEnabled && !root)} onClick={() => void submit()}>
              {props.t('save')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <DirectoryDialog
        open={pickerOpen}
        backendAddress={desktopShell ? props.backends.active.address : draft.backendAddress}
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
