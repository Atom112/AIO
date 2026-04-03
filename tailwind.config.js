/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      colors: {
        // 映射 CSS 变量，使你可以使用 bg-primary, text-primary-50 等类名
        'pri': 'var(--primary-color)',
        'pri-5': 'var(--primary-5)',
        'pri-10': 'var(--primary-10)',
        'pri-20': 'var(--primary-20)',
        'pri-30': 'var(--primary-30)',
        'pri-50': 'var(--primary-50)',
        // 深色背景色阶：统一管理所有深灰色背景
        'dark': {
          DEFAULT: '#1e1e1e',
          50: '#666666',
          100: '#555555',
          200: '#484848',
          300: '#333333',
          400: '#2e2e2e',
          500: '#2a2a2a',
          600: '#252525',
          700: '#222222',
          800: '#1e1e1e',
          850: '#1a1a1a',
          900: '#151515',
          950: '#121212',
        },
        // 危险/关闭操作色
        'danger': '#E08090',
      },
      boxShadow: {
        'theme-inner': 'inset 0 0 20px 1px var(--primary-30)',
      },
      keyframes: {
        'bubble-in': {
          '0%': { opacity: '0', transform: 'translateY(10px) scale(0.98)' },
          '100%': { opacity: '1', transform: 'translateY(0) scale(1)' },
        },
        'cursor-blink': {
          '50%': { opacity: '0' },
        },
        'tagFadeIn': {
          'from': { opacity: '0', transform: 'translateY(5px)' },
          'to': { opacity: '1', transform: 'translateY(0)' },
        },
        'slide-up': {
          '0%': { opacity: '0', transform: 'translateY(12px)' },
          '100%': { opacity: '1', transform: 'translateY(0)' },
        },
        'pulse-glow': {
          '0%, 100%': { opacity: '0.4' },
          '50%': { opacity: '0.8' },
        },
        'message-in': {
          '0%': {
            opacity: '0',
            transform: 'translateY(16px) scale(0.95)',
            filter: 'blur(4px)'
          },
          '100%': {
            opacity: '1',
            transform: 'translateY(0) scale(1)',
            filter: 'blur(0)'
          },
        },
        menuEnter: {
          '0%': {
            opacity: '0',
            transform: 'translateY(-8px) scale(0.95)',
          },
          '100%': {
            opacity: '1',
            transform: 'translateY(0) scale(1)',
          },
        },
        menuExit: {
          '0%': {
            opacity: '1',
            transform: 'translateY(0) scale(1)',
          },
          '100%': {
            opacity: '0',
            transform: 'translateY(-8px) scale(0.95)',
          },
        },
      },
      animation: {
        'message-in': 'message-in 0.4s cubic-bezier(0.2, 0.8, 0.2, 1) forwards',
        'menu-enter': 'menuEnter 0.2s ease-out forwards',
        'menu-exit': 'menuExit 0.2s ease-in forwards',
      },
    },
  },
  plugins: [],
}