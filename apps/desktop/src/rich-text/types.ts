// Promoted from the accepted S04 contract for B04 product use.
export type BoldMark = { type: 'bold' }

export type LinkMark = {
  type: 'link'
  attrs: { href: string }
}

export type RichTextMark = BoldMark | LinkMark

export type TextNode = {
  type: 'text'
  text: string
  marks?: RichTextMark[]
}

export type ImageRefNode = {
  type: 'imageRef'
  attrs: {
    asset_id: string
    display_width?: number
  }
}

export type NoteLinkNode = {
  type: 'noteLink'
  attrs: {
    target_note_id: string
    display_text?: string
  }
}

export type InlineNode = TextNode | ImageRefNode | NoteLinkNode

export type ParagraphNode = {
  type: 'paragraph'
  content?: InlineNode[]
}

export type HeadingNode = {
  type: 'heading'
  attrs: { level: 1 | 2 | 3 | 4 | 5 | 6 }
  content?: InlineNode[]
}

export type CodeBlockNode = {
  type: 'codeBlock'
  content?: Array<{ type: 'text'; text: string }>
}

export type ListItemNode = {
  type: 'listItem'
  content: [ParagraphNode, ...ListBlockNode[]]
}

export type TaskItemNode = {
  type: 'taskItem'
  attrs: { checked: boolean }
  content: [ParagraphNode, ...ListBlockNode[]]
}

export type BulletListNode = {
  type: 'bulletList'
  content: ListItemNode[]
}

export type OrderedListNode = {
  type: 'orderedList'
  attrs?: { start: number }
  content: ListItemNode[]
}

export type TaskListNode = {
  type: 'taskList'
  content: TaskItemNode[]
}

export type ListBlockNode = BulletListNode | OrderedListNode | TaskListNode

export type BlockNode =
  | ParagraphNode
  | HeadingNode
  | CodeBlockNode
  | BulletListNode
  | OrderedListNode
  | TaskListNode

export type RichTextDoc = {
  type: 'doc'
  content: BlockNode[]
}

export type StoredRichTextEnvelope = {
  body_schema_version: number
  body_json: unknown
}

export type FixtureEnvelope = StoredRichTextEnvelope & {
  fixture_id: string
  expected_body_text?: string
  expected_image_refs?: Array<{ asset_id: string; display_width?: number }>
  expected_note_links?: Array<{ target_note_id: string; display_text?: string }>
}


export type RichTextDocument = RichTextDoc
