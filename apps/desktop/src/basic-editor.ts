import { UndoRedo } from "@tiptap/extensions";

import { S04ImageRef, s04Extensions } from "./rich-text/extensions";

export function richTextEditorExtensions(
  loadImage: (assetId: string) => Promise<string> = async () => {
    throw new Error("Image display requires an authorized Note context.");
  },
) {
  return [
    ...s04Extensions.map((extension) => extension.name === "imageRef"
      ? S04ImageRef.configure({ loadImage })
      : extension),
    UndoRedo,
  ];
}

export const RICH_TEXT_EDITOR_EXTENSIONS = richTextEditorExtensions();

// Kept as a compatibility export for the B02 regression surface.
export const BASIC_EDITOR_EXTENSIONS = RICH_TEXT_EDITOR_EXTENSIONS;
