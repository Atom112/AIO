/**
 * Pending-attachment chips (阶段4 第2项拆分切片：ChatInterface <900 目标).
 *
 * Verbatim extraction of the pending-file preview row from ChatInterface: one chip
 * per queued attachment (image thumbnail / file icon + name + remove button).
 * Removing a chip drops the store entry and discards the backend attachment —
 * unless the same file is still queued elsewhere (duplicate-pending guard).
 */
import { For, Show, type Component } from 'solid-js';
import { discardChatAttachment } from '../../../core/api/attachment';
import type { PendingAttachment } from '../../../core/store/store';
import Icon from '../../../shared/components/Icon';

export interface PendingFileChipsProps {
  pendingFiles: () => PendingAttachment[];
  setPendingFiles: (fn: (prev: PendingAttachment[]) => PendingAttachment[]) => void;
}

const PendingFileChips: Component<PendingFileChipsProps> = (props) => {
  return (
    <div class="flex flex-wrap gap-[3px] bg-transparent pt-[3px] relative z-0">
      <For each={props.pendingFiles()}>
        {(file, i) => (
          <div
            class="flex items-center rounded-[16px] text-[12px] px-[10px] py-1 transition-all duration-200"
            style={{
              background: 'rgba(var(--primary-rgb),0.08)',
              border: '1px solid rgba(var(--primary-rgb),0.12)',
              color: 'rgba(var(--primary-rgb),0.6)',
            }}
          >
            <Show
              when={file.type === 'image'}
              fallback={
                <span class="mr-1 inline-flex">
                  <Icon name="file" size={12} class="opacity-60" />
                </span>
              }
            >
              <img
                src={file.previewUrl}
                loading="lazy"
                class="w-5 h-5 object-cover mr-1 rounded-[2px]"
              />
            </Show>
            {file.name}
            <button
              class="flex items-center bg-none border-none text-[rgba(var(--text-base-rgb),0.5)] cursor-pointer text-lg leading-none ml-2 transition-colors duration-200 hover:text-[var(--color-danger)]"
              onClick={() => {
                const currentIndex = i();
                const duplicatePending = props
                  .pendingFiles()
                  .some((pending, index) => index !== currentIndex && pending.id === file.id);
                if (!duplicatePending) {
                  void discardChatAttachment(file.id).catch(console.error);
                }
                props.setPendingFiles((p) => p.filter((_, idx) => idx !== currentIndex));
              }}
            >
              ×
            </button>
          </div>
        )}
      </For>
    </div>
  );
};

export default PendingFileChips;
