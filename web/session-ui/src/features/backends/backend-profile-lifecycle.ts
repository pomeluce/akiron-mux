import { invoke } from '@tauri-apps/api/core';
import type { BackendLifecycleOutcome, BackendProfileIntent } from '@/types';

export function applyBackendProfileIntent(intent: BackendProfileIntent) {
  return invoke<BackendLifecycleOutcome>('apply_backend_profile_intent', { intent });
}

type RefreshResult = BackendLifecycleOutcome | null;

/**
 * Owns active Remote Profile refresh scheduling. A generation prevents a late
 * response from a previous backend from publishing into the current snapshot.
 */
export class BackendProfileRefreshLoop {
  private generation = 0;
  private timer: number | null = null;
  private inFlight = false;

  constructor(
    private readonly intervalMs = 10_000,
    private readonly applyIntent = applyBackendProfileIntent,
  ) {}

  start(profileId: string, publish: (outcome: BackendLifecycleOutcome) => void) {
    this.stop();
    const generation = this.generation;
    const refresh = async () => {
      if (generation !== this.generation || this.inFlight) return;
      this.inFlight = true;
      let outcome: RefreshResult = null;
      try {
        outcome = await this.applyIntent({ type: 'refresh', profileId });
      } catch {
        // Native infrastructure errors are retried, while expected network
        // failures arrive as a typed offline outcome.
      } finally {
        this.inFlight = false;
      }
      if (generation !== this.generation) return;
      if (outcome) publish(outcome);
      if (outcome?.type === 'identityConfirmationRequired' || outcome?.type === 'authenticationRequired') return;
      this.timer = window.setTimeout(refresh, this.intervalMs);
    };
    void refresh();
  }

  stop() {
    this.generation += 1;
    this.inFlight = false;
    if (this.timer !== null) window.clearTimeout(this.timer);
    this.timer = null;
  }
}
