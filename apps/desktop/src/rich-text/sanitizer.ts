// Promoted from the accepted S04 sanitizer and bounded for B04 product use.
import { parseFragment } from 'parse5'
import { isApprovedExternalLink } from './link-policy.js'
import { MAX_BODY_DEPTH, MAX_BODY_NODES, MAX_BODY_JSON_BYTES, utf8BytesWithinLimit } from './limits.js'
import type {
  BlockNode,
  InlineNode,
  ListBlockNode,
  ListItemNode,
  RichTextDoc,
  RichTextMark,
  TaskItemNode,
} from './types.js'

type HtmlAttribute = { name: string; value: string }

type HtmlNode = {
  nodeName: string
  tagName?: string
  value?: string
  attrs?: HtmlAttribute[]
  childNodes?: HtmlNode[]
  content?: HtmlNode
}

export type SanitizeAction = 'REJECT' | 'DOWNGRADE_TO_TEXT' | 'STRIP_ATTRIBUTE'

export type SanitizeEvent = {
  code: string
  action: SanitizeAction
  subject: string
}

export type PasteSanitizeResult =
  | {
      accepted: true
      disposition: 'accepted' | 'downgraded'
      document: RichTextDoc
      events: SanitizeEvent[]
    }
  | {
      accepted: false
      disposition: 'rejected'
      document: null
      events: SanitizeEvent[]
    }

export type PasteDocumentValidator = (document: RichTextDoc) => { valid: boolean }

const uuidPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i
const dangerousUrlPattern = /^(?:javascript|data|file|vbscript):/i
const windowsAbsolutePathPattern = /^(?:[a-z]:[\\/]|\\\\)/i
const canonicalPositiveIntegerPattern = /^[1-9][0-9]*$/
const unicodeLength = (value: string): number => Array.from(value).length

const hardRejectedElements = new Set([
  'script',
  'iframe',
  'style',
  'object',
  'embed',
  'svg',
  'math',
  'video',
  'audio',
  'form',
  'input',
  'button',
  'textarea',
  'select',
  'meta',
  'base',
])

const approvedTags = new Set([
  'p',
  'h1',
  'h2',
  'h3',
  'h4',
  'h5',
  'h6',
  'strong',
  'b',
  'ul',
  'ol',
  'li',
  'pre',
  'code',
  'a',
  'img',
  'span',
  'br',
])

const allowedAttributesByTag: Readonly<Record<string, ReadonlySet<string>>> = {
  ul: new Set(['data-type']),
  ol: new Set(['start']),
  li: new Set(['data-type', 'data-checked']),
  a: new Set(['href', 'data-node', 'data-note-id', 'data-display-text']),
  img: new Set(['src', 'data-asset-id', 'data-display-width']),
  span: new Set(['data-node', 'data-asset-id', 'data-display-width', 'data-note-id', 'data-display-text']),
}

const getAttribute = (node: HtmlNode, name: string): string | undefined =>
  node.attrs?.find((attribute) => attribute.name.toLowerCase() === name.toLowerCase())?.value

// parse5 stores descendants of <template> under a separate DocumentFragment in
// node.content, not under node.childNodes. All security and conversion walkers
// must use this accessor so they observe the same complete tree.
const childrenOf = (node: HtmlNode): HtmlNode[] =>
  node.content?.childNodes ?? node.childNodes ?? []

const htmlTreeBudgetViolation = (root: HtmlNode): SanitizeEvent | undefined => {
  const stack: Array<{ node: HtmlNode; depth: number }> = [{ node: root, depth: 0 }]
  let nodes = 0
  while (stack.length > 0) {
    const current = stack.pop()!
    nodes += 1
    if (nodes > MAX_BODY_NODES) {
      return { code: 'PASTE_HTML_NODE_LIMIT_EXCEEDED', action: 'REJECT', subject: 'paste' }
    }
    if (current.depth > MAX_BODY_DEPTH) {
      return { code: 'PASTE_HTML_DEPTH_LIMIT_EXCEEDED', action: 'REJECT', subject: 'paste' }
    }
    const children = childrenOf(current.node)
    for (let index = children.length - 1; index >= 0; index -= 1) {
      stack.push({ node: children[index], depth: current.depth + 1 })
    }
  }
  return undefined
}

const textContent = (node: HtmlNode): string => {
  if (node.nodeName === '#text') return node.value ?? ''
  return childrenOf(node).map(textContent).join('')
}

const hasAttributePrefix = (node: HtmlNode, prefix: string): boolean =>
  (node.attrs ?? []).some((attribute) => attribute.name.toLowerCase().startsWith(prefix))

const preflight = (root: HtmlNode, events: SanitizeEvent[]): boolean => {
  let rejected = false

  const visit = (node: HtmlNode, parent?: HtmlNode): void => {
    const tag = node.tagName?.toLowerCase()
    if (tag && hardRejectedElements.has(tag)) {
      events.push({ code: 'ACTIVE_CONTENT_ELEMENT', action: 'REJECT', subject: tag })
      rejected = true
    }

    if (tag === 'link') {
      events.push({ code: 'EXTERNAL_RESOURCE_ELEMENT', action: 'REJECT', subject: tag })
      rejected = true
    }

    if (hasAttributePrefix(node, 'on')) {
      events.push({ code: 'EVENT_HANDLER_ATTRIBUTE', action: 'REJECT', subject: tag ?? node.nodeName })
      rejected = true
    }

    if (tag && getAttribute(node, 'style') !== undefined) {
      events.push({ code: 'STYLE_ATTRIBUTE_STRIPPED', action: 'STRIP_ATTRIBUTE', subject: tag })
    }

    if (tag) {
      const allowed = allowedAttributesByTag[tag] ?? new Set<string>()
      for (const attribute of node.attrs ?? []) {
        const name = attribute.name.toLowerCase()
        if (name !== 'style' && !name.startsWith('on') && !allowed.has(name)) {
          events.push({ code: 'ATTRIBUTE_STRIPPED', action: 'STRIP_ATTRIBUTE', subject: `${tag}.${name}` })
        }
      }
    }

    const rawHref = getAttribute(node, 'href')
    const href = rawHref?.trim()
    if (rawHref !== undefined && rawHref !== href) {
      events.push({ code: 'LINK_HREF_TRIMMED', action: 'STRIP_ATTRIBUTE', subject: tag ?? node.nodeName })
    }
    if (href && (dangerousUrlPattern.test(href) || windowsAbsolutePathPattern.test(href))) {
      events.push({ code: 'UNSAFE_LINK_TARGET', action: 'REJECT', subject: href })
      rejected = true
    }
    if (href && unicodeLength(href) > 2048) {
      events.push({ code: 'LINK_HREF_TOO_LONG', action: 'REJECT', subject: tag ?? node.nodeName })
      rejected = true
    }

    const dataType = getAttribute(node, 'data-type')
    if (tag === 'ul' && dataType !== undefined && dataType !== 'taskList') {
      events.push({ code: 'INVALID_LIST_DATA_TYPE', action: 'REJECT', subject: dataType })
      rejected = true
    }
    if (tag === 'li') {
      const parentIsTaskList = parent?.tagName?.toLowerCase() === 'ul'
        && getAttribute(parent, 'data-type') === 'taskList'
      const checked = getAttribute(node, 'data-checked')
      if (parentIsTaskList) {
        if (dataType !== 'taskItem') {
          events.push({ code: 'INVALID_TASK_ITEM_CONTEXT', action: 'REJECT', subject: 'task-list-child' })
          rejected = true
        }
        if (checked !== 'true' && checked !== 'false') {
          events.push({ code: 'INVALID_TASK_ITEM_CHECKED', action: 'REJECT', subject: checked ?? 'missing' })
          rejected = true
        }
      } else if (dataType === 'taskItem' || checked !== undefined) {
        events.push({ code: 'INVALID_TASK_ITEM_CONTEXT', action: 'REJECT', subject: 'non-task-list-parent' })
        rejected = true
      } else if (dataType !== undefined) {
        events.push({ code: 'INVALID_LIST_ITEM_TYPE', action: 'REJECT', subject: dataType })
        rejected = true
      }
    }

    if (tag === 'img') {
      const assetId = getAttribute(node, 'data-asset-id')
      const src = getAttribute(node, 'src')?.trim()
      if (!assetId || !uuidPattern.test(assetId)) {
        events.push({ code: 'UNTRUSTED_IMAGE_SOURCE', action: 'REJECT', subject: src ?? 'img-without-asset-id' })
        rejected = true
      } else if (src && src !== `desktop-notes-asset://${assetId}`) {
        events.push({ code: 'UNTRUSTED_IMAGE_SOURCE', action: 'REJECT', subject: src })
        rejected = true
      }
    }

    for (const child of childrenOf(node)) visit(child, node)
  }

  visit(root)
  return !rejected
}

const blockTags = new Set(['p', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'ul', 'ol', 'pre'])
const listTags = new Set(['ul', 'ol'])

const isNoteLinkElement = (node: HtmlNode): boolean => {
  const tag = node.tagName?.toLowerCase()
  return (tag === 'a' && getAttribute(node, 'data-note-id') !== undefined)
    || (tag === 'span' && getAttribute(node, 'data-node') === 'noteLink')
}

const isImageRefElement = (node: HtmlNode): boolean => {
  const tag = node.tagName?.toLowerCase()
  return tag === 'img' || (tag === 'span' && getAttribute(node, 'data-node') === 'imageRef')
}

const validateListItemGrammar = (node: HtmlNode, events: SanitizeEvent[]): boolean => {
  const logicalBlocks: Array<'paragraph' | 'nested-list' | 'unsupported-heading' | 'unsupported-code'> = []
  let pendingInline = false

  const flushInline = (): void => {
    if (pendingInline) logicalBlocks.push('paragraph')
    pendingInline = false
  }

  for (const child of childrenOf(node)) {
    if (child.nodeName === '#comment') continue
    if (child.nodeName === '#text') {
      if ((child.value ?? '').trim().length > 0) pendingInline = true
      continue
    }

    const tag = child.tagName?.toLowerCase()
    if (tag === 'p') {
      flushInline()
      logicalBlocks.push('paragraph')
    } else if (tag && listTags.has(tag)) {
      flushInline()
      logicalBlocks.push('nested-list')
    } else if (tag && /^h[1-6]$/.test(tag)) {
      flushInline()
      logicalBlocks.push('unsupported-heading')
    } else if (tag === 'pre') {
      flushInline()
      logicalBlocks.push('unsupported-code')
    } else {
      // Inline and unknown inert wrappers are handled by inlineChildren. Even
      // an empty element is tracked so it cannot be silently moved across a
      // block boundary.
      pendingInline = true
    }
  }
  flushInline()

  const unsupported = logicalBlocks.find((block) => block.startsWith('unsupported-'))
  if (unsupported) {
    events.push({
      code: unsupported === 'unsupported-heading'
        ? 'UNSUPPORTED_LIST_ITEM_BLOCK_HEADING'
        : 'UNSUPPORTED_LIST_ITEM_BLOCK_CODEBLOCK',
      action: 'REJECT',
      subject: 'listItem',
    })
    return false
  }

  const paragraphCount = logicalBlocks.filter((block) => block === 'paragraph').length
  if (paragraphCount > 1) {
    events.push({ code: 'MULTI_PARAGRAPH_LIST_ITEM_REJECTED', action: 'REJECT', subject: 'listItem' })
    return false
  }
  if (paragraphCount === 1 && logicalBlocks[0] !== 'paragraph') {
    events.push({ code: 'INVALID_LIST_ITEM_BLOCK_ORDER', action: 'REJECT', subject: 'listItem' })
    return false
  }
  if (paragraphCount === 0 && logicalBlocks.some((block) => block === 'nested-list')) {
    events.push({
      code: 'LIST_ITEM_EMPTY_PARAGRAPH_INSERTED',
      action: 'DOWNGRADE_TO_TEXT',
      subject: 'listItem',
    })
  }
  return true
}

// Validate exactly the HTML grammar that the later converter can represent.
// This is deliberately separate from security preflight and JSON Schema:
// preflight rejects active content, this grammar prevents semantic loss or
// reordering, and Schema validates the resulting persistence document.
const validateConversionGrammar = (root: HtmlNode, events: SanitizeEvent[]): boolean => {
  let rejected = false

  const reject = (code: string, subject: string): void => {
    events.push({ code, action: 'REJECT', subject })
    rejected = true
  }

  const visit = (node: HtmlNode, parent: HtmlNode | undefined, ancestors: HtmlNode[]): void => {
    const tag = node.tagName?.toLowerCase()
    const parentTag = parent?.tagName?.toLowerCase()

    if (parentTag && listTags.has(parentTag)) {
      if (node.nodeName === '#text') {
        if ((node.value ?? '').trim().length > 0) reject('INVALID_LIST_CONTENT', parentTag)
      } else if (node.nodeName !== '#comment' && tag !== 'li') {
        reject('INVALID_LIST_CONTENT', parentTag)
      }
    }

    if (tag === 'li') {
      if (!parentTag || !listTags.has(parentTag)) reject('INVALID_LIST_ITEM_CONTEXT', parentTag ?? 'root')
      if (!validateListItemGrammar(node, events)) rejected = true
    }

    if (tag && blockTags.has(tag) && parentTag && parentTag !== 'li') {
      reject('BLOCK_IN_INLINE_CONTEXT', `${parentTag}>${tag}`)
    }

    if (tag === 'code' && parentTag !== 'pre') {
      events.push({ code: 'INLINE_CODE_DOWNGRADED', action: 'DOWNGRADE_TO_TEXT', subject: parentTag ?? 'root' })
    }
    if (parentTag === 'pre' && tag && tag !== 'code') reject('UNSUPPORTED_CODE_BLOCK_CONTENT', tag)
    const grandparentTag = ancestors.at(-2)?.tagName?.toLowerCase()
    if (parentTag === 'code' && grandparentTag === 'pre' && tag) {
      reject('UNSUPPORTED_CODE_BLOCK_CONTENT', tag)
    }

    const imageRef = isImageRefElement(node)
    const noteLink = isNoteLinkElement(node)
    const atom = imageRef || noteLink
    if (atom) {
      const markedAncestor = ancestors.find((ancestor) => {
        const ancestorTag = ancestor.tagName?.toLowerCase()
        return ancestorTag === 'strong'
          || ancestorTag === 'b'
          || (ancestorTag === 'a' && !isNoteLinkElement(ancestor))
      })
      if (markedAncestor) reject('MARK_ON_ATOM_NOT_SUPPORTED', tag ?? node.nodeName)
    }

    if (imageRef && tag === 'span') {
      const hasMeaningfulChild = childrenOf(node).some((child) =>
        child.nodeName === '#text' ? (child.value ?? '').trim().length > 0 : child.nodeName !== '#comment')
      if (hasMeaningfulChild) reject('IMAGE_REF_NONEMPTY_CONTENT', 'span[data-node=imageRef]')
    }

    if (noteLink) {
      const elementChild = childrenOf(node).some((child) => child.tagName !== undefined)
      if (elementChild) reject('NOTE_LINK_COMPLEX_CONTENT', tag ?? node.nodeName)

      const noteId = getAttribute(node, 'data-note-id') ?? ''
      const explicitDisplay = getAttribute(node, 'data-display-text')
      const visibleText = textContent(node)
      if (explicitDisplay !== undefined) {
        if (explicitDisplay.length > 0 && visibleText !== explicitDisplay) {
          reject('NOTE_LINK_DISPLAY_MISMATCH', noteId || 'missing-note-id')
        } else if (explicitDisplay.length === 0 && visibleText !== '' && visibleText !== noteId) {
          reject('NOTE_LINK_DISPLAY_MISMATCH', noteId || 'missing-note-id')
        }
      } else if (visibleText !== visibleText.trim()) {
        reject('NOTE_LINK_DISPLAY_WHITESPACE_UNSUPPORTED', noteId || 'missing-note-id')
      }

      if (tag === 'a') {
        const href = getAttribute(node, 'href')?.trim()
        if (href && href !== `desktop-notes://note/${noteId}`) {
          reject('NOTE_LINK_HREF_MISMATCH', href)
        }
      }
    }

    if (tag === 'span') {
      const dataNode = getAttribute(node, 'data-node')
      const hasImageMetadata = getAttribute(node, 'data-asset-id') !== undefined
        || getAttribute(node, 'data-display-width') !== undefined
      const hasNoteMetadata = getAttribute(node, 'data-note-id') !== undefined
        || getAttribute(node, 'data-display-text') !== undefined
      if ((hasImageMetadata && dataNode !== 'imageRef') || (hasNoteMetadata && dataNode !== 'noteLink')) {
        reject('INVALID_ATOM_METADATA_CONTEXT', dataNode ?? 'span-without-data-node')
      }
    }

    if (tag === 'a' && !noteLink) {
      const hasNoteMetadata = getAttribute(node, 'data-node') !== undefined
        || getAttribute(node, 'data-display-text') !== undefined
      if (hasNoteMetadata) reject('INVALID_ATOM_METADATA_CONTEXT', 'a-without-note-id')
    } else if (tag === 'a' && noteLink) {
      const dataNode = getAttribute(node, 'data-node')
      if (dataNode !== undefined && dataNode !== 'noteLink') {
        reject('INVALID_ATOM_METADATA_CONTEXT', `a[data-node=${dataNode}]`)
      }
    }

    for (const child of childrenOf(node)) visit(child, node, [...ancestors, node])
  }

  visit(root, undefined, [])
  return !rejected
}

const markKey = (mark: RichTextMark): string =>
  mark.type === 'bold' ? 'bold' : `link:${mark.attrs.href}`

const normalizeInline = (nodes: InlineNode[]): InlineNode[] => {
  const output: InlineNode[] = []
  for (const node of nodes) {
    if (node.type === 'text' && node.text.length === 0) continue
    const previous = output.at(-1)
    if (
      previous?.type === 'text' &&
      node.type === 'text' &&
      JSON.stringify(previous.marks ?? []) === JSON.stringify(node.marks ?? [])
    ) {
      previous.text += node.text
    } else {
      output.push(node)
    }
  }
  return output
}

const inlineChildren = (
  node: HtmlNode,
  inheritedMarks: RichTextMark[],
  events: SanitizeEvent[],
): InlineNode[] => {
  const output: InlineNode[] = []

  const visit = (child: HtmlNode, marks: RichTextMark[]): void => {
    if (child.nodeName === '#text') {
      const value = child.value ?? ''
      if (value) output.push(marks.length > 0 ? { type: 'text', marks, text: value } : { type: 'text', text: value })
      return
    }

    const tag = child.tagName?.toLowerCase()
    if (!tag) {
      for (const nested of childrenOf(child)) visit(nested, marks)
      return
    }

    if (tag === 'code') {
      const value = textContent(child)
      if (value) {
        output.push(marks.length > 0 ? { type: 'text', marks, text: value } : { type: 'text', text: value })
      }
      return
    }

    if (tag === 'strong' || tag === 'b') {
      const nextMarks = marks.some((mark) => mark.type === 'bold') ? marks : [...marks, { type: 'bold' as const }]
      for (const nested of childrenOf(child)) visit(nested, nextMarks)
      return
    }

    if (tag === 'a') {
      const noteId = getAttribute(child, 'data-note-id')
      if (noteId !== undefined) {
        if (marks.length > 0) throw new Error('MARK_ON_ATOM_NOT_SUPPORTED')
        if (!uuidPattern.test(noteId)) throw new Error('INVALID_NOTE_LINK_ID')
        const display = getAttribute(child, 'data-display-text') ?? textContent(child).trim()
        if (unicodeLength(display) > 500) throw new Error('NOTE_LINK_DISPLAY_TOO_LONG')
        output.push({
          type: 'noteLink',
          attrs: {
            target_note_id: noteId,
            ...(display ? { display_text: display } : {}),
          },
        })
        return
      }

      const href = getAttribute(child, 'href')?.trim()
      if (!href || !isApprovedExternalLink(href)) throw new Error('INVALID_EXTERNAL_LINK')
      if (unicodeLength(href) > 2048) throw new Error('LINK_HREF_TOO_LONG')
      const nextMarks: RichTextMark[] = marks.filter((mark) => mark.type !== 'link')
      nextMarks.push({ type: 'link', attrs: { href } })
      const outputLengthBeforeLink = output.length
      for (const nested of childrenOf(child)) visit(nested, nextMarks)
      if (output.length === outputLengthBeforeLink) throw new Error('EMPTY_EXTERNAL_LINK_REJECTED')
      return
    }

    if (tag === 'img' || (tag === 'span' && getAttribute(child, 'data-node') === 'imageRef')) {
      if (marks.length > 0) throw new Error('MARK_ON_ATOM_NOT_SUPPORTED')
      const assetId = getAttribute(child, 'data-asset-id')
      if (!assetId || !uuidPattern.test(assetId)) throw new Error('INVALID_IMAGE_REF_ID')
      const widthText = getAttribute(child, 'data-display-width')
      if (widthText !== undefined && !canonicalPositiveIntegerPattern.test(widthText)) {
        throw new Error('INVALID_IMAGE_REF_WIDTH')
      }
      const width = widthText === undefined ? undefined : Number(widthText)
      if (width !== undefined && (!Number.isSafeInteger(width) || width < 48 || width > 4096)) {
        throw new Error('INVALID_IMAGE_REF_WIDTH')
      }
      output.push({
        type: 'imageRef',
        attrs: { asset_id: assetId, ...(width !== undefined ? { display_width: width } : {}) },
      })
      return
    }

    if (tag === 'span' && getAttribute(child, 'data-node') === 'noteLink') {
      if (marks.length > 0) throw new Error('MARK_ON_ATOM_NOT_SUPPORTED')
      const noteId = getAttribute(child, 'data-note-id')
      if (!noteId || !uuidPattern.test(noteId)) throw new Error('INVALID_NOTE_LINK_ID')
      const display = getAttribute(child, 'data-display-text') ?? textContent(child).trim()
      if (unicodeLength(display) > 500) throw new Error('NOTE_LINK_DISPLAY_TOO_LONG')
      output.push({
        type: 'noteLink',
        attrs: { target_note_id: noteId, ...(display ? { display_text: display } : {}) },
      })
      return
    }

    if (tag === 'span' && getAttribute(child, 'data-node')) {
      events.push({
        code: 'UNKNOWN_ELEMENT_DOWNGRADED',
        action: 'DOWNGRADE_TO_TEXT',
        subject: `span[data-node=${getAttribute(child, 'data-node')}]`,
      })
    }

    if (tag === 'br') {
      events.push({ code: 'HARD_BREAK_DOWNGRADED', action: 'DOWNGRADE_TO_TEXT', subject: 'br' })
      output.push(marks.length > 0 ? { type: 'text', marks, text: '\n' } : { type: 'text', text: '\n' })
      return
    }

    if (!approvedTags.has(tag) || ['h1', 'h2', 'h3', 'p', 'ul', 'ol', 'li', 'pre'].includes(tag)) {
      if (!approvedTags.has(tag)) {
        events.push({ code: 'UNKNOWN_ELEMENT_DOWNGRADED', action: 'DOWNGRADE_TO_TEXT', subject: tag })
      }
      for (const nested of childrenOf(child)) visit(nested, marks)
      return
    }

    for (const nested of childrenOf(child)) visit(nested, marks)
  }

  for (const child of childrenOf(node)) visit(child, inheritedMarks)
  return normalizeInline(output).map((inline) => {
    if (inline.type !== 'text' || !inline.marks) return inline
    const deduplicated = [...new Map(inline.marks.map((mark) => [markKey(mark), mark])).values()]
    return { ...inline, marks: deduplicated }
  })
}

const paragraphFrom = (node: HtmlNode, events: SanitizeEvent[]): BlockNode => {
  const content = inlineChildren(node, [], events)
  return content.length > 0 ? { type: 'paragraph', content } : { type: 'paragraph' }
}

const listItemFrom = (node: HtmlNode, events: SanitizeEvent[]): ListItemNode => {
  const blocks = blockChildren(node, events)
  const unsupported = blocks.find((block) => ['heading', 'codeBlock'].includes(block.type))
  if (unsupported) throw new Error(`UNSUPPORTED_LIST_ITEM_BLOCK_${unsupported.type.toUpperCase()}`)

  const paragraphs = blocks.filter((block) => block.type === 'paragraph')
  if (paragraphs.length > 1) throw new Error('MULTI_PARAGRAPH_LIST_ITEM_REJECTED')

  const nested = blocks.filter(
    (block): block is ListBlockNode => ['bulletList', 'orderedList', 'taskList'].includes(block.type),
  )
  if (paragraphs.length === 0) {
    return { type: 'listItem', content: [{ type: 'paragraph' }, ...nested] }
  }
  if (blocks[0]?.type !== 'paragraph') throw new Error('INVALID_LIST_ITEM_BLOCK_ORDER')

  return {
    type: 'listItem',
    content: [paragraphs[0] as ListItemNode['content'][0], ...nested],
  }
}

const taskItemFrom = (node: HtmlNode, events: SanitizeEvent[]): TaskItemNode => {
  const base = listItemFrom(node, events)
  const checked = getAttribute(node, 'data-checked') === 'true'
  return { type: 'taskItem', attrs: { checked }, content: base.content }
}

const directListItems = (node: HtmlNode): HtmlNode[] => {
  const items: HtmlNode[] = []
  for (const child of childrenOf(node)) {
    if (child.nodeName === '#text') {
      if ((child.value ?? '').trim().length > 0) throw new Error('INVALID_LIST_CONTENT')
      continue
    }
    if (child.nodeName === '#comment') continue
    if (child.tagName?.toLowerCase() !== 'li') throw new Error('INVALID_LIST_CONTENT')
    items.push(child)
  }
  if (items.length === 0) throw new Error('EMPTY_LIST_REJECTED')
  return items
}

const elementToBlock = (node: HtmlNode, events: SanitizeEvent[]): BlockNode[] => {
  const tag = node.tagName?.toLowerCase()
  if (!tag) return []

  if (tag === 'p') return [paragraphFrom(node, events)]
  if (/^h[1-6]$/.test(tag)) {
    const content = inlineChildren(node, [], events)
    const level = Number(tag.slice(1)) as 1 | 2 | 3 | 4 | 5 | 6
    return [content.length > 0 ? { type: 'heading', attrs: { level }, content } : { type: 'heading', attrs: { level } }]
  }
  if (tag === 'pre') {
    const text = textContent(node)
    return [text ? { type: 'codeBlock', content: [{ type: 'text', text }] } : { type: 'codeBlock' }]
  }
  if (tag === 'ul') {
    const items = directListItems(node)
    if (getAttribute(node, 'data-type') === 'taskList') {
      return [{ type: 'taskList', content: items.map((item) => taskItemFrom(item, events)) }]
    }
    return [{ type: 'bulletList', content: items.map((item) => listItemFrom(item, events)) }]
  }
  if (tag === 'ol') {
    const items = directListItems(node)
    const startText = getAttribute(node, 'start')
    if (startText !== undefined && !canonicalPositiveIntegerPattern.test(startText)) {
      throw new Error('INVALID_ORDERED_LIST_START')
    }
    const start = startText === undefined ? 1 : Number(startText)
    if (!Number.isSafeInteger(start) || start < 1 || start > 1_000_000) throw new Error('INVALID_ORDERED_LIST_START')
    return [{
      type: 'orderedList',
      ...(start === 1 ? {} : { attrs: { start } }),
      content: items.map((item) => listItemFrom(item, events)),
    }]
  }

  if (!approvedTags.has(tag)) {
    events.push({ code: 'UNKNOWN_ELEMENT_DOWNGRADED', action: 'DOWNGRADE_TO_TEXT', subject: tag })
  }
  const inline = inlineChildren(node, [], events)
  return inline.length > 0 ? [{ type: 'paragraph', content: inline }] : []
}

function blockChildren(root: HtmlNode, events: SanitizeEvent[]): BlockNode[] {
  const blocks: BlockNode[] = []
  let pendingInline: InlineNode[] = []
  let pendingLeadingWhitespace = ''

  const flushInline = (): void => {
    const content = normalizeInline(pendingInline)
    if (content.length > 0) blocks.push({ type: 'paragraph', content })
    pendingInline = []
    pendingLeadingWhitespace = ''
  }

  for (const child of childrenOf(root)) {
    if (child.nodeName === '#text') {
      const value = child.value ?? ''
      if (value.trim().length > 0) {
        if (pendingLeadingWhitespace) pendingInline.push({ type: 'text', text: pendingLeadingWhitespace })
        pendingLeadingWhitespace = ''
        pendingInline.push({ type: 'text', text: value })
      } else if (pendingInline.length > 0) {
        pendingInline.push({ type: 'text', text: value })
      } else {
        pendingLeadingWhitespace += value
      }
      continue
    }
    const tag = child.tagName?.toLowerCase()
    if (tag && ['p', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'ul', 'ol', 'pre'].includes(tag)) {
      pendingLeadingWhitespace = ''
      flushInline()
      blocks.push(...elementToBlock(child, events))
    } else {
      if (pendingLeadingWhitespace) pendingInline.push({ type: 'text', text: pendingLeadingWhitespace })
      pendingLeadingWhitespace = ''
      pendingInline.push(...inlineChildren({ nodeName: '#document-fragment', childNodes: [child] }, [], events))
    }
  }
  flushInline()
  return blocks
}

export const sanitizePastedHtml = (html: string): PasteSanitizeResult => {
  const events: SanitizeEvent[] = []

  if (!utf8BytesWithinLimit(html, MAX_BODY_JSON_BYTES)) {
    events.push({ code: 'PASTE_HTML_BYTES_EXCEEDED', action: 'REJECT', subject: 'paste' })
    return { accepted: false, disposition: 'rejected', document: null, events }
  }

  const root = parseFragment(html) as unknown as HtmlNode
  const budgetViolation = htmlTreeBudgetViolation(root)
  if (budgetViolation) {
    events.push(budgetViolation)
    return { accepted: false, disposition: 'rejected', document: null, events }
  }

  if (!preflight(root, events)) {
    return { accepted: false, disposition: 'rejected', document: null, events }
  }
  if (!validateConversionGrammar(root, events)) {
    return { accepted: false, disposition: 'rejected', document: null, events }
  }

  try {
    const content = blockChildren(root, events)
    const document: RichTextDoc = {
      type: 'doc',
      content: content.length > 0 ? content : [{ type: 'paragraph' }],
    }
    const downgraded = events.some((event) => event.action !== 'REJECT')
    return { accepted: true, disposition: downgraded ? 'downgraded' : 'accepted', document, events }
  } catch (error) {
    events.push({
      code: error instanceof Error ? error.message : 'SANITIZER_TRANSFORM_FAILED',
      action: 'REJECT',
      subject: 'paste',
    })
    return { accepted: false, disposition: 'rejected', document: null, events }
  }
}

// Persistence callers must use this composition, not sanitizer output alone.
// Mirroring schema constraints above improves event precision, while this
// mandatory final gate remains fail-closed if either contract later evolves.
export const sanitizePastedHtmlForPersistence = (
  html: string,
  validateDocument: PasteDocumentValidator,
): PasteSanitizeResult => {
  const result = sanitizePastedHtml(html)
  if (!result.accepted) return result
  if (validateDocument(result.document).valid) return result
  return {
    accepted: false,
    disposition: 'rejected',
    document: null,
    events: [
      ...result.events,
      { code: 'SCHEMA_VALIDATION_FAILED', action: 'REJECT', subject: 'sanitized-document' },
    ],
  }
}
