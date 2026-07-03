// Highlight #tags and @mentions inside blip editors without adding any
// ProseMirror node types — a pure view-layer decoration plugin. Keeping the
// document schema-plain matters: the blip fragment stays wire-compatible
// with the server (which parses the same raw text) and with every other
// client, regardless of who has this extension loaded.

import { Extension } from '@tiptap/core'
import { Plugin } from '@tiptap/pm/state'
import { Decoration, DecorationSet } from '@tiptap/pm/view'

// Unicode-aware: tags/mentions may contain letters from any script.
const TAG_RE = /(^|[^\p{L}\p{N}_])(#[\p{L}\p{N}_-]+)/gu
const MENTION_RE = /(^|[^\p{L}\p{N}_])(@[\p{L}\p{N}][\p{L}\p{N}_.-]*)/gu

export const TagsMentions = Extension.create({
  name: 'tagsMentions',
  addProseMirrorPlugins() {
    return [
      new Plugin({
        props: {
          decorations(state) {
            const decos: Decoration[] = []
            state.doc.descendants((node, pos) => {
              if (!node.isText || !node.text) return
              const text = node.text
              for (const re of [TAG_RE, MENTION_RE]) {
                re.lastIndex = 0
                let m: RegExpExecArray | null
                while ((m = re.exec(text)) !== null) {
                  const token = m[2]
                  const start = pos + m.index + m[1].length
                  const end = start + token.length
                  const isTag = token.startsWith('#')
                  const agent = !isTag && token.toLowerCase() === '@assistant'
                  decos.push(
                    Decoration.inline(start, end, {
                      class: isTag
                        ? 'pw-tag'
                        : agent
                          ? 'pw-mention pw-mention-agent'
                          : 'pw-mention',
                      'data-token': token,
                    }),
                  )
                }
              }
            })
            return DecorationSet.create(state.doc, decos)
          },
        },
      }),
    ]
  },
})
