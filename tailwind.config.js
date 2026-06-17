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
        // 深色背景色阶：改为偏蓝灰调
        'dark': {
          DEFAULT: '#161a28',
          50: '#6a6e7a',
          100: '#585c6a',
          200: '#464a58',
          300: '#353946',
          400: '#2d3140',
          500: '#262a38',
          600: '#1f2331',
          700: '#1a1e2c',
          800: '#161a28',
          850: '#121624',
          900: '#0e121f',
          950: '#0a0e1a',
        },
        // 危险/关闭操作色
        'danger': '#E08090',
      },
      boxShadow: {
        'theme-inner': 'inset 0 0 20px rgba(0, 0, 0, 0.3)',
        'acrylic': '0 8px 32px rgba(0, 0, 0, 0.25)',
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
