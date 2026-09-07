import { afterEach, describe, expect, it, vi } from "vitest";

import {
  SaveAcknowledgementGate,
  SaveCoordinator,
  type EditableNoteDraft,
  type SaveState,
} from "./save-coordinator";

const firstDraft: EditableNoteDraft = {
  noteId: "b30256a2-a873-4589-aadc-d8e63fc7fb4d",
  noteDate: "2026-09-06",
  title: "First",
  bodyJson: { type: "doc", content: [{ type: "paragraph" }] },
  baseRevision: 1,
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

describe("B02 save coordinator", () => {
  afterEach(() => vi.useRealTimers());

  it("debounces ordinary edits for 750 ms and Ctrl+S flushes immediately", async () => {
    vi.useFakeTimers();
    const states: SaveState[] = [];
    const save = vi.fn().mockResolvedValue({ revision: 2 });
    const coordinator = new SaveCoordinator(save, (state) => states.push(state), () => undefined);

    coordinator.changed(firstDraft);
    expect(states.at(-1)).toBe("Dirty");
    await vi.advanceTimersByTimeAsync(749);
    expect(save).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(save).toHaveBeenCalledTimes(1);
    expect(states).toContain("Saving");
    expect(states.at(-1)).toBe("Saved");

    coordinator.changed({ ...firstDraft, title: "Second", baseRevision: 2 });
    await coordinator.flush();
    expect(save).toHaveBeenCalledTimes(2);
    coordinator.dispose();
  });

  it("resets the 750 ms debounce whenever typing continues", async () => {
    vi.useFakeTimers();
    const save = vi.fn().mockResolvedValue({ revision: 2 });
    const coordinator = new SaveCoordinator(save, () => undefined, () => undefined);
    coordinator.changed(firstDraft);
    await vi.advanceTimersByTimeAsync(500);
    coordinator.changed({ ...firstDraft, title: "Still typing" });
    await vi.advanceTimersByTimeAsync(749);
    expect(save).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(save).toHaveBeenCalledTimes(1);
    expect(save.mock.calls[0][0].title).toBe("Still typing");
    coordinator.dispose();
  });

  it("does not let an old save response mark newer text Saved", async () => {
    vi.useFakeTimers();
    const first = deferred<{ revision: number }>();
    const second = deferred<{ revision: number }>();
    const save = vi.fn()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);
    const states: SaveState[] = [];
    const revisions: number[] = [];
    const coordinator = new SaveCoordinator(
      save,
      (state) => states.push(state),
      (revision) => revisions.push(revision),
    );

    coordinator.changed(firstDraft);
    await vi.advanceTimersByTimeAsync(750);
    coordinator.changed({ ...firstDraft, title: "Newer text" });
    first.resolve({ revision: 2 });
    await first.promise;
    await vi.runAllTimersAsync();

    expect(states.at(-1)).not.toBe("Saved");
    expect(save.mock.calls[1][0].title).toBe("Newer text");
    expect(save.mock.calls[1][0].baseRevision).toBe(2);

    second.resolve({ revision: 3 });
    await second.promise;
    await vi.runAllTimersAsync();
    expect(revisions).toEqual([2, 3]);
    expect(states.at(-1)).toBe("Saved");
    coordinator.dispose();
  });

  it("reports Error when persistence rejects", async () => {
    const states: SaveState[] = [];
    const coordinator = new SaveCoordinator(
      vi.fn().mockRejectedValue(new Error("synthetic")),
      (state) => states.push(state),
      () => undefined,
    );
    coordinator.changed(firstDraft);
    await coordinator.flush();
    expect(states.at(-1)).toBe("Error");
    expect(coordinator.hasUnsavedChanges()).toBe(true);
    coordinator.dispose();
  });

  it("keeps a newer in-memory edit recoverable when an older in-flight save fails", async () => {
    const first = deferred<{ revision: number }>();
    const save = vi.fn()
      .mockReturnValueOnce(first.promise)
      .mockResolvedValueOnce({ revision: 2 });
    const states: SaveState[] = [];
    const coordinator = new SaveCoordinator(save, (state) => states.push(state), () => undefined);
    coordinator.changed(firstDraft);
    const firstFlush = coordinator.flush();
    coordinator.changed({ ...firstDraft, title: "Keep this newer draft" });
    first.reject(new Error("synthetic failure"));
    await expect(firstFlush).resolves.toBe(false);
    expect(states.at(-1)).toBe("Error");
    expect(coordinator.hasUnsavedChanges()).toBe(true);

    await expect(coordinator.flush()).resolves.toBe(true);
    expect(save.mock.calls[1][0].title).toBe("Keep this newer draft");
    expect(states.at(-1)).toBe("Saved");
    coordinator.dispose();
  });

  it("ignores response 1 after response 2 instead of rolling revision backward", () => {
    const gate = new SaveAcknowledgementGate(1);
    gate.register("save-1", 1);
    gate.register("save-2", 2);
    expect(gate.accept("save-2", 3)).toEqual({ accepted: true, revision: 3 });
    expect(gate.accept("save-1", 2)).toEqual({ accepted: false, revision: 3 });
    expect(gate.currentRevision).toBe(3);
  });

  it("forgets a failed change id so a late acknowledgement cannot be accepted", () => {
    const gate = new SaveAcknowledgementGate(1);
    gate.register("failed-save", 1);
    gate.reject("failed-save");
    expect(gate.accept("failed-save", 2)).toEqual({ accepted: false, revision: 1 });
  });

  it("waits for the newest queued edit before allowing a date transition", async () => {
    vi.useFakeTimers();
    const first = deferred<{ revision: number }>();
    const second = deferred<{ revision: number }>();
    const save = vi.fn()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);
    const coordinator = new SaveCoordinator(save, () => undefined, () => undefined);

    coordinator.changed(firstDraft);
    await vi.advanceTimersByTimeAsync(750);
    coordinator.changed({ ...firstDraft, title: "Newest pending title" });
    let transitionFinished = false;
    const transition = coordinator.flush().then((result) => {
      transitionFinished = true;
      return result;
    });

    first.resolve({ revision: 2 });
    await first.promise;
    await Promise.resolve();
    expect(transitionFinished).toBe(false);
    expect(save).toHaveBeenCalledTimes(2);
    expect(save.mock.calls[1][0]).toMatchObject({
      title: "Newest pending title",
      baseRevision: 2,
    });

    second.resolve({ revision: 3 });
    await expect(transition).resolves.toBe(true);
    coordinator.rebase("2026-09-10", 4);
    coordinator.changed({ ...firstDraft, noteDate: "2026-09-10", baseRevision: 4, title: "After move" });
    expect(save).toHaveBeenCalledTimes(2);
    coordinator.dispose();
  });
});
