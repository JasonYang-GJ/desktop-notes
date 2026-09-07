export interface StoredRichTextEnvelope {
  body_schema_version: number;
  body_json: unknown;
}

export type RichTextMigration = (input: StoredRichTextEnvelope) => StoredRichTextEnvelope;

export type RichTextLoadResult =
  | {
      mode: "editable";
      envelope: StoredRichTextEnvelope;
      original_preserved: true;
      applied: string[];
    }
  | {
      mode: "read-only";
      original_envelope: StoredRichTextEnvelope;
      original_preserved: true;
      applied: string[];
      error_code: "RICH_TEXT_VERSION_UNSUPPORTED" | "RICH_TEXT_MIGRATION_FAILED";
      error_detail: string;
    };

const clone = <T>(value: T): T => structuredClone(value);

export class RichTextMigrationRegistry {
  constructor(
    private readonly targetVersion: number,
    private readonly migrations: ReadonlyMap<number, RichTextMigration> = new Map(),
  ) {}

  load(input: StoredRichTextEnvelope): RichTextLoadResult {
    const original = clone(input);
    const applied: string[] = [];
    let current = clone(input);

    if (!Number.isSafeInteger(current.body_schema_version)
      || current.body_schema_version < 0
      || current.body_schema_version > this.targetVersion) {
      return {
        mode: "read-only",
        original_envelope: original,
        original_preserved: true,
        applied,
        error_code: "RICH_TEXT_VERSION_UNSUPPORTED",
        error_detail: `Stored rich-text version v${String(current.body_schema_version)} is not supported.`,
      };
    }

    try {
      while (current.body_schema_version < this.targetVersion) {
        const from = current.body_schema_version;
        const migration = this.migrations.get(from);
        if (!migration) throw new Error(`No migration registered from v${from}.`);
        current = migration(clone(current));
        if (current.body_schema_version !== from + 1) {
          throw new Error(`Migration v${from} did not produce v${from + 1}.`);
        }
        applied.push(`v${from}->v${current.body_schema_version}`);
      }
      return {
        mode: "editable",
        envelope: current,
        original_preserved: true,
        applied,
      };
    } catch (error) {
      return {
        mode: "read-only",
        original_envelope: original,
        original_preserved: true,
        applied,
        error_code: "RICH_TEXT_MIGRATION_FAILED",
        error_detail: error instanceof Error ? error.message : String(error),
      };
    }
  }
}
