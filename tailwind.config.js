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
        primary: 'var(--primary-color)',
        'primary-5': 'var(--primary-5)',
        'primary-10': 'var(--primary-10)',
        'primary-20': 'var(--primary-20)',
        'primary-30': 'var(--primary-30)',
        'primary-50': 'var(--primary-50)',
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