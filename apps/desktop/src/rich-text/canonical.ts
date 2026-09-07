// Promoted from the accepted S04 contract for B04 product use.
import type {
  BlockNode,
  ImageRefNode,
  InlineNode,
  NoteLinkNode,
  RichTextDoc,
  RichTextMark,
} from './types.js'

const markRank: Record<RichTextMark['type'], number> = { bold: 0, link: 1 }

const cloneMark = (mark: RichTextMark): RichTextMark =>
  mark.type === 'bold'
    ? { type: 'bold' }
    : { type: 'link', attrs: { href: mark.attrs.href } }

const canonicalMarks = (marks: RichTextMark[] | undefined): RichTextMark[] | undefined => {
  if (!marks || marks.length === 0) return undefined
  return [...marks]
    .map(cloneMark)
    .sort((left, right) => markRank[left.type] - markRank[right.type])
}

const canonicalInline = (node: InlineNode): InlineNode => {
  if (node.type === 'text') {
    const marks = canonicalMarks(node.marks)
    return marks
      ? { type: 'text', marks, text: node.text }
      : { type: 'text', text: node.text }
  }

  if (node.type === 'imageRef') {
    const attrs: ImageRefNode['attrs'] = { asset_id: node.attrs.asset_id }
    if (node.attrs.display_width !== undefined && node.attrs.display_width !== null) {
      attrs.display_width = node.attrs.display_width
    }
    return { type: 'imageRef', attrs }
  }

  const attrs: NoteLinkNode['attrs'] = { target_note_id: node.attrs.target_note_id }
  if (node.attrs.display_text !== undefined && node.attrs.display_text !== null && node.attrs.display_text !== '') {
    attrs.display_text = node.attrs.display_text
  }
  return { type: 'noteLink', attrs }
}

const canonicalBlock = (node: BlockNode): BlockNode => {
  switch (node.type) {
    case 'paragraph': {
      const content = node.content?.map(canonicalInline)
      return content && content.length > 0 ? { type: 'paragraph', content } : { type: 'paragraph' }
    }
    case 'heading': {
      const content = node.content?.map(canonicalInline)
      return content && content.length > 0
        ? { type: 'heading', attrs: { level: node.attrs.level }, content }
        : { type: 'heading', attrs: { level: node.attrs.level } }
    }
    case 'codeBlock': {
      const content = node.content?.map((child) => ({ type: 'text' as const, text: child.text }))
      return content && content.length > 0 ? { type: 'codeBlock', content } : { type: 'codeBlock' }
    }
    case 'bulletList':
      return {
        type: 'bulletList',
        content: node.content.map((item) => ({
          type: 'listItem',
          content: item.content.map(canonicalBlock) as typeof item.content,
        })),
      }
    case 'orderedList': {
      const attrs = node.attrs && node.attrs.start !== 1 ? { start: node.attrs.start } : undefined
      return {
        type: 'orderedList',
        ...(attrs ? { attrs } : {}),
        content: node.content.map((item) => ({
          type: 'listItem',
          content: item.content.map(canonicalBlock) as typeof item.content,
        })),
      }
    }
    case 'taskList':
      return {
        type: 'taskList',
        content: node.content.map((item) => ({
          type: 'taskItem',
          attrs: { checked: item.attrs.checked },
          content: item.content.map(canonicalBlock) as typeof item.content,
        })),
      }
  }
}

export const canonicalize = (document: RichTextDoc): RichTextDoc => ({
  type: 'doc',
  content: document.content.map(canonicalBlock),
})

export const canonicalStringify = (document: RichTextDoc): string =>
  JSON.stringify(canonicalize(document))
