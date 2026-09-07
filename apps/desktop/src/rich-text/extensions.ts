// Promoted from the accepted S04 contract for B04 product use.
import { Mark, Node } from '@tiptap/core'
import { isApprovedExternalLink } from './link-policy.js'

const isSafeExternalHref = (value: unknown): value is string =>
  typeof value === 'string' && isApprovedExternalLink(value)

const readInteger = (value: string | null, fallback: number): number => {
  if (value === null) return fallback
  const parsed = Number.parseInt(value, 10)
  return Number.isSafeInteger(parsed) ? parsed : fallback
}

export const S04Document = Node.create({
  name: 'doc',
  topNode: true,
  content: 'block+',
})

export const S04Paragraph = Node.create({
  name: 'paragraph',
  group: 'block',
  content: 'inline*',
  parseHTML: () => [{ tag: 'p' }],
  renderHTML: () => ['p', 0],
})

export const S04Text = Node.create({
  name: 'text',
  group: 'inline',
})

export const S04Heading = Node.create({
  name: 'heading',
  group: 'block',
  content: 'inline*',
  defining: true,
  addAttributes() {
    return {
      level: {
        default: 1,
        parseHTML: (element) => readInteger(element.tagName.slice(1), 1),
      },
    }
  },
  parseHTML: () => [1, 2, 3, 4, 5, 6].map((level) => ({ tag: `h${level}` })),
  renderHTML({ node }) {
    const level = [1, 2, 3, 4, 5, 6].includes(node.attrs.level as number) ? node.attrs.level : 1
    return [`h${level}`, 0]
  },
})

export const S04BulletList = Node.create({
  name: 'bulletList',
  group: 'block listBlock',
  content: 'listItem+',
  parseHTML: () => [{ tag: 'ul:not([data-type="taskList"])' }],
  renderHTML: () => ['ul', 0],
})

export const S04OrderedList = Node.create({
  name: 'orderedList',
  group: 'block listBlock',
  content: 'listItem+',
  addAttributes() {
    return {
      start: {
        default: 1,
        parseHTML: (element) => readInteger(element.getAttribute('start'), 1),
      },
    }
  },
  parseHTML: () => [{ tag: 'ol' }],
  renderHTML({ node }) {
    const start = node.attrs.start as number
    return start === 1 ? ['ol', 0] : ['ol', { start }, 0]
  },
})

export const S04ListItem = Node.create({
  name: 'listItem',
  content: 'paragraph listBlock*',
  parseHTML: () => [{ tag: 'li:not([data-type="taskItem"])' }],
  renderHTML: () => ['li', 0],
  addKeyboardShortcuts() {
    return {
      Enter: () => this.editor.commands.splitListItem(this.name),
      Tab: () => this.editor.commands.sinkListItem(this.name),
      'Shift-Tab': () => this.editor.commands.liftListItem(this.name),
    }
  },
})

export const S04TaskList = Node.create({
  name: 'taskList',
  group: 'block listBlock',
  content: 'taskItem+',
  parseHTML: () => [{ tag: 'ul[data-type="taskList"]' }],
  renderHTML: () => ['ul', { 'data-type': 'taskList' }, 0],
})

export const S04TaskItem = Node.create({
  name: 'taskItem',
  content: 'paragraph listBlock*',
  addAttributes() {
    return {
      checked: {
        default: false,
        parseHTML: (element) => element.getAttribute('data-checked') === 'true',
      },
    }
  },
  parseHTML: () => [{ tag: 'li[data-type="taskItem"]' }],
  addKeyboardShortcuts() {
    return {
      Enter: () => this.editor.commands.splitListItem(this.name, { checked: false }),
      Tab: () => this.editor.commands.sinkListItem(this.name),
      'Shift-Tab': () => this.editor.commands.liftListItem(this.name),
    }
  },
  renderHTML({ node }) {
    return [
      'li',
      {
        'data-type': 'taskItem',
        'data-checked': node.attrs.checked ? 'true' : 'false',
      },
      0,
    ]
  },
  addNodeView() {
    return ({ node: initialNode, getPos, view }) => {
      let node = initialNode
      const dom = document.createElement('li')
      dom.setAttribute('data-type', 'taskItem')
      const checkbox = document.createElement('input')
      checkbox.type = 'checkbox'
      checkbox.checked = Boolean(node.attrs.checked)
      checkbox.setAttribute('aria-label', 'Toggle checklist item')
      checkbox.contentEditable = 'false'
      const contentDOM = document.createElement('div')
      contentDOM.className = 'task-item-content'
      dom.append(checkbox, contentDOM)

      const change = (): void => {
        const position = getPos()
        if (typeof position !== 'number') return
        view.dispatch(view.state.tr.setNodeMarkup(position, undefined, {
          ...node.attrs,
          checked: checkbox.checked,
        }))
      }
      checkbox.addEventListener('change', change)

      return {
        dom,
        contentDOM,
        update(updatedNode) {
          if (updatedNode.type.name !== 'taskItem') return false
          node = updatedNode
          checkbox.checked = Boolean(node.attrs.checked)
          return true
        },
        stopEvent: (event) => event.target === checkbox,
        destroy: () => checkbox.removeEventListener('change', change),
      }
    }
  },
})

export const S04CodeBlock = Node.create({
  name: 'codeBlock',
  group: 'block',
  content: 'text*',
  marks: '',
  code: true,
  defining: true,
  parseHTML: () => [{ tag: 'pre', preserveWhitespace: 'full' }],
  renderHTML: () => ['pre', ['code', 0]],
})

interface ImageRefOptions {
  loadImage: (assetId: string) => Promise<string>
}

export const S04ImageRef = Node.create<ImageRefOptions>({
  name: 'imageRef',
  group: 'inline',
  inline: true,
  atom: true,
  selectable: true,
  addOptions() {
    return {
      loadImage: async () => {
        throw new Error('Image display is unavailable outside an authorized Note context.')
      },
    }
  },
  addAttributes() {
    return {
      asset_id: {
        default: null,
        parseHTML: (element) => element.getAttribute('data-asset-id'),
      },
      display_width: {
        default: undefined,
        parseHTML: (element) => {
          const value = element.getAttribute('data-display-width')
          return value === null ? null : readInteger(value, 0)
        },
      },
    }
  },
  parseHTML: () => [
    { tag: 'span[data-node="imageRef"]' },
    { tag: 'img[data-asset-id]' },
  ],
  renderHTML({ node }) {
    const attrs: Record<string, string> = {
      'data-node': 'imageRef',
      'data-asset-id': String(node.attrs.asset_id),
    }
    if (node.attrs.display_width !== null && node.attrs.display_width !== undefined) {
      attrs['data-display-width'] = String(node.attrs.display_width)
    }
    return ['span', attrs]
  },
  addNodeView() {
    return ({ node: initialNode }) => {
      let node = initialNode
      let generation = 0
      let destroyed = false
      const dom = document.createElement('span')
      dom.className = 'image-ref image-ref-loading'
      dom.contentEditable = 'false'
      const image = document.createElement('img')
      image.alt = 'Encrypted Note image'
      image.draggable = false
      const status = document.createElement('span')
      status.className = 'image-ref-status'
      status.textContent = 'Opening encrypted image…'
      dom.append(image, status)

      const load = (): void => {
        const current = ++generation
        const assetId = typeof node.attrs.asset_id === 'string' ? node.attrs.asset_id : ''
        dom.dataset.assetId = assetId
        const width = node.attrs.display_width
        image.style.width = Number.isInteger(width) ? `${String(width)}px` : ''
        image.removeAttribute('src')
        dom.className = 'image-ref image-ref-loading'
        status.textContent = 'Opening encrypted image…'
        void this.options.loadImage(assetId).then((url) => {
          if (destroyed || current !== generation) return
          image.src = url
          dom.className = 'image-ref image-ref-ready'
          status.textContent = ''
        }).catch(() => {
          if (destroyed || current !== generation) return
          dom.className = 'image-ref image-ref-error'
          status.textContent = 'Encrypted image unavailable'
        })
      }
      load()

      return {
        dom,
        update(updatedNode) {
          if (updatedNode.type.name !== 'imageRef') return false
          const changed = updatedNode.attrs.asset_id !== node.attrs.asset_id
            || updatedNode.attrs.display_width !== node.attrs.display_width
          node = updatedNode
          if (changed) load()
          return true
        },
        destroy() {
          destroyed = true
          generation += 1
          image.removeAttribute('src')
        },
      }
    }
  },
})

export const S04NoteLink = Node.create({
  name: 'noteLink',
  group: 'inline',
  inline: true,
  atom: true,
  selectable: true,
  addAttributes() {
    return {
      target_note_id: {
        default: null,
        parseHTML: (element) => element.getAttribute('data-note-id'),
      },
      display_text: {
        default: null,
        parseHTML: (element) => element.getAttribute('data-display-text') ?? element.textContent,
      },
    }
  },
  parseHTML: () => [
    { tag: 'a[data-note-id]' },
    { tag: 'span[data-node="noteLink"]' },
  ],
  renderHTML({ node }) {
    const target = String(node.attrs.target_note_id)
    const display = node.attrs.display_text ? String(node.attrs.display_text) : target
    const attrs: Record<string, string> = {
      'data-node': 'noteLink',
      'data-note-id': target,
      'data-display-text': node.attrs.display_text ? String(node.attrs.display_text) : '',
      href: `desktop-notes://note/${target}`,
    }
    return ['a', attrs, display]
  },
})

export const S04Bold = Mark.create({
  name: 'bold',
  parseHTML: () => [{ tag: 'strong' }, { tag: 'b' }],
  renderHTML: () => ['strong', 0],
})

export const S04Link = Mark.create({
  name: 'link',
  inclusive: false,
  addAttributes() {
    return {
      href: {
        default: null,
        parseHTML: (element) => {
          const href = element.getAttribute('href')
          return isSafeExternalHref(href) ? href : false
        },
      },
    }
  },
  parseHTML: () => [{ tag: 'a[href]:not([data-note-id])' }],
  renderHTML({ HTMLAttributes }) {
    return ['a', { href: HTMLAttributes.href }, 0]
  },
})

export const s04Extensions = [
  S04Document,
  S04Paragraph,
  S04Text,
  S04Heading,
  S04BulletList,
  S04OrderedList,
  S04ListItem,
  S04TaskList,
  S04TaskItem,
  S04CodeBlock,
  S04ImageRef,
  S04NoteLink,
  S04Bold,
  S04Link,
]
