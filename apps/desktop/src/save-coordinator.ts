import type { TiptapDocument } from "./notes";

export const AUTOSAVE_DEBOUNCE_MS = 750;
export type SaveState = "Dirty" | "Saving" | "Saved" | "Error";

export interface EditableNoteDraft {
  noteId: string;
  noteDate: string;
  title: string;
  bodyJson: TiptapDocument;
  baseRevision: number;
}

interface SaveAcknowledgement {
  revision: number;
}

type SaveOperation = (
  draft: EditableNoteDraft,
  clientChangeId: string,
) => Promise<SaveAcknowledgement>;

export class SaveAcknowledgementGate {
  private readonly pending = new Map<string, number>();
  private latestAcceptedSequence = 0;

  constructor(private revision: number) {}

  get currentRevision(): number {
    return this.revision;
  }

  register(clientChangeId: string, sequence: number): void {
    this.pending.set(clientChangeId, sequence);
  }

  reject(clientChangeId: string): void {
    this.pending.delete(clientChangeId);
  }

  accept(clientChangeId: string, revision: number): { accepted: boolean; revision: number } {
    const sequence = this.pending.get(clientChangeId);
    this.pending.delete(clientChangeId);
    if (sequence === undefined
      || sequence < this.latestAcceptedSequence
      || !Number.isSafeInteger(revision)
      || revision <= this.revision) {
      return { accepted: false, revision: this.revision };
    }
    this.latestAcceptedSequence = sequence;
    this.revision = revision;
    return { accepted: true, revision };
  }

  advanceTo(revision: number): void {
    if (Number.isSafeInteger(revision) && revision > this.revision) this.revision = revision;
  }
}

export class SaveCoordinator {
  private draft?: EditableNoteDraft;
  private editVersion = 0;
  private timer?: ReturnType<typeof setTimeout>;
  private drain?: Promise<boolean>;
  private dirty = false;
  private disposed = false;
  private readonly acknowledgements = new SaveAcknowledgementGate(0);

  constructor(
    private readonly save: SaveOperation,
    private readonly onState: (state: SaveState) => void,
    private readonly onRevision: (revision: number) => void,
  ) {}

  changed(draft: EditableNoteDraft): void {
    if (this.disposed) return;
    this.draft = { ...draft };
    this.acknowledgements.advanceTo(draft.baseRevision);
    this.editVersion += 1;
    this.dirty = true;
    this.onState("Dirty");
    this.clearTimer();
    if (this.drain) return;
    this.timer = setTimeout(() => {
      this.timer = undefined;
      void this.flush();
    }, AUTOSAVE_DEBOUNCE_MS);
  }

  hasUnsavedChanges(): boolean {
    return this.dirty || this.drain !== undefined;
  }

  async flush(): Promise<boolean> {
    if (this.disposed) return false;
    this.clearTimer();
    if (!this.draft || !this.dirty) {
      return this.drain ?? true;
    }
    if (this.drain) return this.drain;

    const drain = this.drainSaves();
    this.drain = drain;
    try {
      return await drain;
    } finally {
      if (this.drain === drain) this.drain = undefined;
    }
  }

  rebase(noteDate: string, revision: number): void {
    if (this.disposed || !this.draft) return;
    this.draft = { ...this.draft, noteDate, baseRevision: revision };
    this.acknowledgements.advanceTo(revision);
    this.dirty = false;
  }

  dispose(): void {
    this.disposed = true;
    this.clearTimer();
  }

  private async drainSaves(): Promise<boolean> {
    while (!this.disposed && this.draft && this.dirty) {
      const capturedVersion = this.editVersion;
      const capturedDraft = { ...this.draft };
      const clientChangeId = crypto.randomUUID();
      this.acknowledgements.register(clientChangeId, capturedVersion);
      this.dirty = false;
      this.onState("Saving");
      try {
        const acknowledgement = await this.save(capturedDraft, clientChangeId);
        if (this.disposed || !this.draft) return false;
        const accepted = this.acknowledgements.accept(clientChangeId, acknowledgement.revision);
        if (!accepted.accepted) {
          this.dirty = true;
          this.onState("Error");
          return false;
        }
        this.draft = { ...this.draft, baseRevision: accepted.revision };
        this.onRevision(accepted.revision);
        if (this.editVersion === capturedVersion && !this.dirty) {
          this.onState("Saved");
          return true;
        }
        this.onState("Dirty");
      } catch {
        this.acknowledgements.reject(clientChangeId);
        if (!this.disposed) {
          this.dirty = true;
          this.onState("Error");
        }
        return false;
      }
    }
    return !this.disposed;
  }

  private clearTimer(): void {
    if (this.timer !== undefined) {
      clearTimeout(this.timer);
      this.timer = undefined;
    }
  }
}
