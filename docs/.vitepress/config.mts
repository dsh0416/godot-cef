import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'Godot CEF',
  description: 'High-performance Chromium Embedded Framework integration for Godot Engine',
  base: '/godot-cef/',

  themeConfig: {
    nav: [
      { text: 'Home', link: '/' },
      { text: 'API Reference', link: '/api/' },
      { text: 'GitHub', link: 'https://github.com/dsh0416/godot-cef' }
    ],

    sidebar: {
      '/api/': [
        {
          text: 'API Reference',
          items: [
            { text: 'Overview', link: '/api/' },
            { text: 'Properties', link: '/api/properties' },
            { text: 'Methods', link: '/api/methods' },
            { text: 'Signals', link: '/api/signals' },
            { text: 'Audio Capture', link: '/api/audio-capture' },
            { text: 'IME Support', link: '/api/ime-support' },
            { text: 'Drag and Drop', link: '/api/drag-and-drop' },
            { text: 'Vulkan Support', link: '/api/vulkan-support' },
            { text: 'GPU Device Pinning', link: '/api/gpu-device-pinning' }
          ]
        }
      ]
    },

    socialLinks: [
      { icon: 'github', link: 'https://github.com/dsh0416/godot-cef' }
    ]
  },

  vite: {
    optimizeDeps: {
      exclude: [
        '@nolebase/vitepress-plugin-enhanced-readabilities/client',
      ]
    },
    ssr: {
      noExternal: [
        '@nolebase/vitepress-plugin-enhanced-readabilities',
      ]
    }
  },
})
