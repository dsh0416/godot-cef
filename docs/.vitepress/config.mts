import { defineConfig } from 'vitepress'
import { withMermaid } from 'vitepress-plugin-mermaid'

export default withMermaid(defineConfig({
  title: 'Godot CEF',
  description: 'High-performance Chromium Embedded Framework integration for Godot Engine',
  base: '/godot-cef/',

  locales: {
    root: {
      label: 'English',
      lang: 'en',
    },
    zh_CN: {
      label: '简体中文',
      lang: 'zh-CN',
      themeConfig: {
        nav: [
          { text: '首页', link: '/zh_CN/' },
          { text: 'API 参考', link: '/zh_CN/api/' },
          { text: 'GitHub', link: 'https://github.com/dsh0416/godot-cef' }
        ],

        sidebar: {
          '/zh_CN/api/': [
            {
              text: 'API 参考',
              items: [
                { text: '概述', link: '/zh_CN/api/' },
                { text: '属性', link: '/zh_CN/api/properties' },
                { text: '方法', link: '/zh_CN/api/methods' },
                { text: '信号', link: '/zh_CN/api/signals' },
                { text: '音频捕获', link: '/zh_CN/api/audio-capture' },
                { text: '输入法支持', link: '/zh_CN/api/ime-support' },
                { text: '拖放', link: '/zh_CN/api/drag-and-drop' },
                { text: '下载', link: '/zh_CN/api/downloads' },
                { text: 'Vulkan 支持', link: '/zh_CN/api/vulkan-support' },
                { text: 'GPU 设备绑定', link: '/zh_CN/api/gpu-device-pinning' }
              ]
            }
          ]
        },

        outline: {
          label: '本页目录'
        },

        docFooter: {
          prev: '上一页',
          next: '下一页'
        },

        lastUpdated: {
          text: '最后更新于'
        },

        returnToTopLabel: '返回顶部',
        sidebarMenuLabel: '菜单',
        darkModeSwitchLabel: '外观',
      }
    }
  },

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
            { text: 'Downloads', link: '/api/downloads' },
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
      ],
      include: [
        'mermaid',
      ]
    },
    ssr: {
      noExternal: [
        '@nolebase/vitepress-plugin-enhanced-readabilities',
      ]
    }
  },

  mermaid: {
    // Mermaid configuration options
  },
}))
