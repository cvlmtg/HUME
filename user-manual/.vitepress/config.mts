import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'HUME',
  description: 'User manual for HUME — a modal text editor for the terminal',

  base: '/HUME/',
  srcDir: 'docs',
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
    ],

    sidebar: [
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
          { text: 'Selections', link: '/selections' },
          { text: 'Language Servers', link: '/lsp' },
        ],
      },
      {
        text: 'Files & Syntax',
        items: [
          { text: 'Files & Buffers', link: '/files-and-buffers' },
          { text: 'Syntax Highlighting', link: '/syntax-highlighting' },
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
        text: 'Customization',
        items: [
          { text: 'Configuration', link: '/configuration' },
          { text: 'Plugins', link: '/plugins' },
          { text: 'Core Plugins', link: '/core-plugins' },
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
    ],

    socialLinks: [
      { icon: 'github', link: 'https://github.com/cvlmtg/HUME' },
    ],

    footer: {
      message: 'Released under the MIT License.',
    },

    search: { provider: 'local' },
  },
})
