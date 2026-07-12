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
          '0%': { opacity: '0', transform: 'translateY(-8px) scale(0.95)' },
          '100%': { opacity: '1', transform: 'translateY(0) scale(1)' },
        },
        menuExit: {
          '0%': { opacity: '1', transform: 'translateY(0) scale(1)' },
          '100%': { opacity: '0', transform: 'translateY(-8px) scale(0.95)' },
        },
        'fade-in': {
          '0%': { opacity: '0', transform: 'scale(0.9)' },
          '100%': { opacity: '1', transform: 'scale(1)' },
        },
        // ======= Moved from index.css =======
        contextMenuIn: {
          from: { opacity: '0', transform: 'scale(0.94) translateY(-4px)' },
          to: { opacity: '1', transform: 'scale(1) translateY(0)' },
        },
        contextMenuOut: {
          from: { opacity: '1', transform: 'scale(1) translateY(0)' },
          to: { opacity: '0', transform: 'scale(0.96) translateY(-2px)' },
        },
        modalOverlayIn: {
          from: { opacity: '0' },
          to: { opacity: '1' },
        },
        modalIn: {
          from: { opacity: '0', transform: 'translateY(12px) scale(0.96)' },
          to: { opacity: '1', transform: 'translateY(0) scale(1)' },
        },
        toastIn: {
          from: { opacity: '0', transform: 'translate(-50%, 12px) scale(0.95)' },
          to: { opacity: '1', transform: 'translate(-50%, 0) scale(1)' },
        },
        rowIn: {
          from: { opacity: '0', transform: 'translateY(6px)' },
          to: { opacity: '1', transform: 'translateY(0)' },
        },
        dropdownIn: {
          from: { opacity: '0', transform: 'translateY(-6px) scale(0.96)' },
          to: { opacity: '1', transform: 'translateY(0) scale(1)' },
        },
        thinkIconPulse: {
          '0%, 100%': { opacity: '0.7' },
          '50%': { opacity: '1' },
        },
        thinkIconSpin: {
          from: { transform: 'rotate(0deg)' },
          to: { transform: 'rotate(360deg)' },
        },
        agentProcessIconPulse: {
          '0%, 100%': { opacity: '0.7' },
          '50%': { opacity: '1' },
        },
        agentProcessIconSpin: {
          from: { transform: 'rotate(0deg)' },
          to: { transform: 'rotate(360deg)' },
        },
        stepCardPulse: {
          '0%, 100%': { borderLeftColor: 'var(--step-border-color)' },
          '50%': { borderLeftColor: 'rgba(var(--primary-rgb), 0.8)' },
        },
        reasoningPopupIn: {
          from: { opacity: '0', transform: 'translateY(6px) scale(0.96)' },
          to: { opacity: '1', transform: 'translateY(0) scale(1)' },
        },
        toolApprovalIn: {
          from: { opacity: '0', transform: 'translateY(-8px)' },
          to: { opacity: '1', transform: 'translateY(0)' },
        },
        commandPaletteFadeIn: {
          from: { opacity: '0', backdropFilter: 'blur(0)' },
          to: { opacity: '1', backdropFilter: 'blur(6px)' },
        },
        commandPaletteSlideIn: {
          from: { opacity: '0', transform: 'translateY(-8px) scale(0.97)' },
          to: { opacity: '1', transform: 'translateY(0) scale(1)' },
        },
        shortcutRecordingPulse: {
          '0%, 100%': { boxShadow: '0 0 0 0 rgba(var(--primary-rgb), 0.3)' },
          '50%': { boxShadow: '0 0 0 6px rgba(var(--primary-rgb), 0)' },
        },
        slashMenuIn: {
          from: { opacity: '0', transform: 'translateY(4px)' },
          to: { opacity: '1', transform: 'translateY(0)' },
        },
        welcomeLogoFloat: {
          '0%, 100%': { transform: 'translateY(0)' },
          '50%': { transform: 'translateY(-8px)' },
        },
        welcomeTextFadeIn: {
          from: { opacity: '0', transform: 'translateY(12px)' },
          to: { opacity: '1', transform: 'translateY(0)' },
        },
        welcomeCardIn: {
          from: { opacity: '0', transform: 'translateY(10px)' },
          to: { opacity: '1', transform: 'translateY(0)' },
        },
      },
      animation: {
        'message-in': 'message-in 0.4s cubic-bezier(0.2, 0.8, 0.2, 1) forwards',
        'menu-enter': 'menuEnter 0.2s ease-out forwards',
        'menu-exit': 'menuExit 0.2s ease-in forwards',
        'fade-in': 'fade-in 0.2s ease-out forwards',
        // ======= Moved from index.css =======
        'context-menu-in': 'contextMenuIn 0.18s cubic-bezier(0.2, 0.8, 0.2, 1) forwards',
        'context-menu-out': 'contextMenuOut 0.14s ease-in forwards',
        'modal-overlay-in': 'modalOverlayIn 0.2s ease forwards',
        'modal-in': 'modalIn 0.25s cubic-bezier(0.2, 0.8, 0.2, 1) forwards',
        'toast-in': 'toastIn 0.3s cubic-bezier(0.175, 0.885, 0.32, 1.275) forwards',
        'row-in': 'rowIn 0.32s cubic-bezier(0.2, 0.8, 0.2, 1) backwards',
        'dropdown-in': 'dropdownIn 0.15s cubic-bezier(0.2, 0.8, 0.2, 1) forwards',
        'think-icon-pulse': 'thinkIconPulse 2s ease-in-out infinite',
        'think-icon-spin': 'thinkIconSpin 2.4s linear infinite',
        'agent-process-pulse': 'agentProcessIconPulse 2s ease-in-out infinite',
        'agent-process-spin': 'agentProcessIconSpin 2.4s linear infinite',
        'step-card-pulse': 'stepCardPulse 2s ease-in-out infinite',
        'reasoning-popup-in': 'reasoningPopupIn 0.18s cubic-bezier(0.2, 0.8, 0.2, 1) forwards',
        'tool-approval-in': 'toolApprovalIn 0.2s ease-out',
        'cmd-palette-fade-in': 'commandPaletteFadeIn 0.12s ease-out',
        'cmd-palette-slide-in': 'commandPaletteSlideIn 0.15s cubic-bezier(0.16, 1, 0.3, 1)',
        'shortcut-recording-pulse': 'shortcutRecordingPulse 1.2s ease-in-out infinite',
        'slash-menu-in': 'slashMenuIn 0.1s ease-out',
        'welcome-logo-float': 'welcomeLogoFloat 3s ease-in-out infinite',
        'welcome-text-fade-in': 'welcomeTextFadeIn 0.5s ease-out both',
        'welcome-card-in': 'welcomeCardIn 0.4s cubic-bezier(0.2, 0.8, 0.2, 1) backwards',
      },
    },
  },
  plugins: [],
}
