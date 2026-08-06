import { existsSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vitepress'
import type { DefaultTheme } from 'vitepress'

// Two channels are deployed from this one config: the latest tagged release
// (default, served at the site root) and nightly (built from the working
// tree, served under /nightly/). CI sets these env vars per build; `npm run
// dev`/`npm run build` with none set reproduces today's single-channel site.
const channel = process.env.DOCS_CHANNEL ?? 'nightly'
const srcDir = process.env.DOCS_SRC ?? 'docs'
const outDir = process.env.DOCS_OUT ?? '.vitepress/dist'
const base = process.env.DOCS_BASE ?? '/HUME/'
const releaseTag = process.env.DOCS_RELEASE_TAG ?? 'latest release'

// The channel you're currently on links internally (base-relative, correct
// in dev and in either deployed build); the other channel has no local
// build to point at, so it always links to the deployed site.
const releaseLink = channel === 'release' ? '/' : 'https://cvlmtg.github.io/HUME/'
const nightlyLink = channel === 'nightly' ? '/' : 'https://cvlmtg.github.io/HUME/nightly/'

const root = fileURLToPath(new URL('..', import.meta.url))

// The release build pairs main's sidebar (this file) with an older tag's
// docs/, so a page added since the tag would otherwise dangle. Prune it.
function pageExists(link: string): boolean {
  const file = link === '/' ? 'index.md' : `${link.slice(1)}.md`
  return existsSync(resolve(root, srcDir, file))
}

function pruneSidebar(items: DefaultTheme.SidebarItem[]): DefaultTheme.SidebarItem[] {
  return items.flatMap((item) => {
    if (item.items) {
      const pruned = pruneSidebar(item.items)
      return pruned.length > 0 ? [{ ...item, items: pruned }] : []
    }
    return !item.link || pageExists(item.link) ? [item] : []
  })
}

export default defineConfig({
  title: 'HUME',
  description: 'User manual for HUME — a modal text editor for the terminal',

  base,
  srcDir,
  outDir,
  // light + dark toggle (default). Use 'dark' to default to dark mode.
  appearance: true,

  // Load the theme fonts. (Or self-host them under /public for offline builds.)
  head: [
    ['link', { rel: 'preconnect', href: 'https://fonts.googleapis.com' }],
    ['link', { rel: 'preconnect', href: 'https://fonts.gstatic.com', crossorigin: '' }],
    ['link', {
      rel: 'stylesheet',
      href: 'https://fonts.googleapis.com/css2?family=Newsreader:ital,wght@0,400;0,500;0,600;1,400;1,500&family=IBM+Plex+Mono:wght@400;500;600&family=IBM+Plex+Sans:wght@400;500;600&display=swap',
    }],
  ],

  themeConfig: {
    nav: [
      { text: 'Manual', link: '/' },
      {
        text: channel === 'release' ? releaseTag : 'nightly',
        items: [
          { text: releaseTag, link: releaseLink, target: '_self', noIcon: true },
          { text: 'nightly', link: nightlyLink, target: '_self', noIcon: true },
        ],
      },
    ],

    sidebar: pruneSidebar([
      { text: 'Home', link: '/' },
      {
        text: 'Getting Started',
        items: [
          { text: 'Installation', link: '/installation' },
          { text: 'Getting Started', link: '/getting-started' },
          { text: 'Modes', link: '/modes' },
        ],
      },
      {
        text: 'Editing',
        items: [
          { text: 'Moving Around', link: '/moving-around' },
          { text: 'Editing', link: '/editing' },
          { text: 'Copy & Paste', link: '/copy-and-paste' },
          { text: 'Selections', link: '/selections' },
          { text: 'Language Servers', link: '/lsp' },
        ],
      },
      {
        text: 'Files & Syntax',
        items: [
          { text: 'Files & Buffers', link: '/files-and-buffers' },
          { text: 'Fuzzy Finder', link: '/pickers' },
          { text: 'Syntax Highlighting', link: '/syntax-highlighting' },
        ],
      },
      {
        text: 'Customization',
        items: [
          { text: 'Configuration', link: '/configuration' },
          { text: 'Core Plugins', link: '/core-plugins' },
          { text: 'Plugins', link: '/plugins' },
        ],
      },
      {
        text: 'Reference',
        items: [
          { text: 'Command-line Flags', link: '/cli' },
          { text: 'Commands', link: '/commands' },
          { text: 'Key Reference', link: '/key-reference' },
        ],
      },
      {
        text: 'Coming From',
        items: [
          { text: 'Vim / Neovim', link: '/from-vim' },
          { text: 'Helix', link: '/from-helix' },
          { text: 'Kakoune', link: '/from-kakoune' },
        ],
      },
      { text: 'Roadmap', link: '/roadmap' },
    ]),

    socialLinks: [
      { icon: 'github', link: 'https://github.com/cvlmtg/HUME' },
    ],

    footer: {
      message: 'Released under the MIT License.',
    },

    search: { provider: 'local' },

    // Consumed by theme/Layout.vue to render the nightly banner.
    channel,
    releaseTag,
  } satisfies DefaultTheme.Config & { channel: string; releaseTag: string },
})
