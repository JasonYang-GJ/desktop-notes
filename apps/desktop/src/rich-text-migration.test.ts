import { describe, expect, it } from "vitest";

import legacyV1 from "../../../tests/fixtures/rich-text/legacy-v1-minimal.json";
import { RichTextMigrationRegistry } from "./rich-text/migration";

describe("RichTextMigrationRegistry", () => {
  it("loads current and historical v1 content without rewriting the source", () => {
    const registry = new RichTextMigrationRegistry(1);
    const source = structuredClone(legacyV1);
    const before = JSON.stringify(source);
    const result = registry.load({
      body_schema_version: source.body_schema_version,
      body_json: source.body_json,
    });

    expect(result.mode).toBe("editable");
    expect(JSON.stringify(source)).toBe(before);
    if (result.mode === "editable") {
      expect(result.envelope.body_schema_version).toBe(1);
      expect(result.applied).toEqual([]);
    }
  });

  it("keeps future or unknown versions read-only and preserves the original envelope", () => {
    const registry = new RichTextMigrationRegistry(1);
    const future = { body_schema_version: 2, body_json: { type: "future", secret: "keep" } };
    const result = registry.load(future);
    expect(result.mode).toBe("read-only");
    if (result.mode === "read-only") {
      expect(result.error_code).toBe("RICH_TEXT_VERSION_UNSUPPORTED");
      expect(result.original_envelope).toEqual(future);
    }
  });

  it("fails closed without mutating the original when a registered migration throws", () => {
    const source = { body_schema_version: 0, body_json: { legacy: "keep exactly" } };
    const registry = new RichTextMigrationRegistry(1, new Map([
      [0, () => { throw new Error("simulated migration failure"); }],
    ]));
    const result = registry.load(source);
    expect(result.mode).toBe("read-only");
    expect(source).toEqual({ body_schema_version: 0, body_json: { legacy: "keep exactly" } });
    if (result.mode === "read-only") {
      expect(result.error_code).toBe("RICH_TEXT_MIGRATION_FAILED");
      expect(result.original_envelope).toEqual(source);
    }
  });
});
