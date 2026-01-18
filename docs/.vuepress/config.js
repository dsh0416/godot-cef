import { viteBundler } from '@vuepress/bundler-vite'
import { defaultTheme } from '@vuepress/theme-default'
import { defineUserConfig } from 'vuepress'

export default defineUserConfig({
  bundler: viteBundler(),
  theme: defaultTheme({
    navbar: [
      {
        text: 'Home',
        link: '/',
      },
      {
        text: 'API Reference',
        link: '/api/',
      },
      {
        text: 'GitHub',
        link: 'https://github.com/dsh0416/godot-cef',
      },
    ],
    sidebar: {
      '/api/': [
        {
          text: 'API Reference',
          children: [
            '/api/',
            '/api/properties',
            '/api/methods',
            '/api/signals',
            '/api/ime-support',
          ],
        },
      ],
    },
  }),
  title: 'Godot CEF',
  description: 'High-performance Chromium Embedded Framework integration for Godot Engine',
  base: '/godot-cef/',
})
