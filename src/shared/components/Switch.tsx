import type { Component } from 'solid-js';

interface SwitchProps {
    checked: boolean;
    onChange: (checked: boolean) => void;
    disabled?: boolean;
    label?: string;
}

const Switch: Component<SwitchProps> = (props) => (
    <button
        type="button"
        role="switch"
        aria-checked={props.checked}
        aria-label={props.label}
        disabled={props.disabled}
        class="relative inline-flex h-5 w-9 shrink-0 items-center rounded-full border transition-colors duration-200 focus:outline-none focus:ring-2 focus:ring-pri-50/40 disabled:cursor-not-allowed disabled:opacity-50"
        classList={{
            'bg-pri border-pri': props.checked,
            'bg-dark-200 border-dark-50': !props.checked,
        }}
        onClick={() => props.onChange(!props.checked)}
    >
        <span
            class="pointer-events-none block h-3.5 w-3.5 rounded-full bg-white shadow-sm transition-transform duration-200"
            classList={{
                'translate-x-[17px]': props.checked,
                'translate-x-[3px]': !props.checked,
            }}
        />
    </button>
);

export default Switch;
