// Markdown rendering (FR-32..34): CommonMark + tables via markdown-it,
// sanitized with DOMPurify, code blocks highlighted with shiki (lazy-loaded
// so the highlighter's grammars don't weigh down the main bundle).

import MarkdownIt from 'markdown-it'
import DOMPurify from 'dompurify'

const md = new MarkdownIt({
  html: false, // raw HTML in markdown is ignored, not parsed
  linkify: true,
  typographer: true,
})

export function renderMarkdown(source: string): string {
  return DOMPurify.sanitize(md.render(source))
}

let highlighter: Promise<typeof import('shiki')> | null = null

/// Progressive enhancement: highlight <pre><code> blocks in-place after the
/// sanitized HTML is mounted.
export async function highlightCodeBlocks(root: HTMLElement) {
  const blocks = root.querySelectorAll('pre code[class*="language-"]')
  if (blocks.length === 0) return
  highlighter ??= import('shiki')
  const shiki = await highlighter
  for (const block of blocks) {
    const lang = /language-(\w+)/.exec(block.className)?.[1] ?? 'text'
    try {
      const html = await shiki.codeToHtml(block.textContent ?? '', {
        lang,
        theme: 'solarized-light',
      })
      const pre = block.parentElement
      if (pre) pre.outerHTML = DOMPurify.sanitize(html)
    } catch {
      // Unknown language: leave the plain block.
    }
  }
}

/// Render a historical yrs XML fragment string (from playback) as safe HTML.
export function sanitizeFragmentXml(xml: string): string {
  return DOMPurify.sanitize(xml, {
    ALLOWED_TAGS: [
      'paragraph',
      'p',
      'strong',
      'em',
      'u',
      's',
      'code',
      'pre',
      'blockquote',
      'h1',
      'h2',
      'h3',
      'ul',
      'ol',
      'li',
      'br',
    ],
    ALLOWED_ATTR: [],
  })
    .replace(/<paragraph>/g, '<p>')
    .replace(/<\/paragraph>/g, '</p>')
}
