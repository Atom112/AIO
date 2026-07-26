import { Component, Show } from 'solid-js';
import Markdown from '../../../shared/components/Markdown';
import Icon from '../../../shared/components/Icon';

export interface BtwOverlayItem {
    id: string;
    question: string;
    answer: string | null;
    loading: boolean;
}

interface BtwResponseBoxProps {
    item: BtwOverlayItem;
    onDismiss: (id: string) => void;
}

const BtwResponseBox: Component<BtwResponseBoxProps> = (props) => {
    return (
        <div class="rounded-lg overflow-hidden w-[85%] self-center my-2"
             style={{
                 border: '1px solid rgba(255,255,255,0.08)',
                 background: 'rgba(200,180,255,0.03)',
             }}
        >
            <div class="flex items-center gap-2 px-3 py-2"
                 style={{
                     'border-bottom': '1px solid rgba(255,255,255,0.04)',
                     color: 'rgba(255,255,255,0.5)',
                     'font-size': '13px',
                 }}
            >
                <span class="font-medium px-1.5 py-px rounded text-[12px]"
                      style={{
                          background: 'rgba(124,154,191,0.12)',
                          color: 'rgba(124,154,191,0.8)',
                          border: '1px solid rgba(124,154,191,0.15)',
                      }}
                >BTW</span>
                <span class="flex-1" style="color: rgba(255,255,255,0.6);">{props.item.question || '(空)'}</span>
                <button
                    onClick={() => props.onDismiss(props.item.id)}
                    class="flex items-center justify-center w-5 h-5 rounded hover:bg-white/10 transition-colors"
                    style="color: rgba(255,255,255,0.3);"
                    title="关闭"
                >
                    <Icon name="x" class="w-3 h-3" />
                </button>
            </div>

            <div class="px-3 py-2 text-[14px] leading-relaxed text-white/85">
                <Show when={props.item.answer} fallback={
                    <Show when={props.item.loading} fallback={
                        <span style="color: rgba(255,255,255,0.3); font-style: italic;">等待回答...</span>
                    }>
                        <span style="color: rgba(255,255,255,0.4);" class="animate-pulse">AI 正在思考中...</span>
                    </Show>
                }>
                    <Markdown content={props.item.answer!} />
                </Show>
            </div>
        </div>
    );
};

export default BtwResponseBox;
