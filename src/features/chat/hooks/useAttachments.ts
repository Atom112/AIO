/**
 * Chat attachment staging (阶段4 第2项拆分切片：ChatPage 按 useAttachments 路径).
 *
 * Pre-validates the extension whitelist (hardening: avoids pointless backend calls),
 * stores the picked file through the attachment API (content extraction + copy into
 * the app attachment dir) and stages the pending entry with an image preview URL.
 * The picker itself stays with its call sites (shortcut command / drag-drop listener).
 */
import { convertFileSrc } from '@tauri-apps/api/core';
import { storeChatAttachment } from '../../../core/api/attachment';
import { t } from '../../../core/i18n';
import type { PendingAttachment } from '../../../core/store/types';

/** Deps the ChatPage component provides. */
export interface AttachmentsDeps {
  setPendingFiles: (fn: (prev: PendingAttachment[]) => PendingAttachment[]) => void;
  setIsProcessing: (v: boolean) => void;
}

const ALLOWED_IMG = ['png', 'jpg', 'jpeg', 'webp'];
const ALLOWED_DOC = [
  'pdf',
  'docx',
  'pptx',
  'txt',
  'md',
  'json',
  'csv',
  'log',
  'xml',
  'yaml',
  'yml',
  'ini',
  'tsv',
  // source code / text
  'rs',
  'c',
  'h',
  'cpp',
  'hpp',
  'cc',
  'cxx',
  'cs',
  'go',
  'java',
  'rb',
  'py',
  'js',
  'mjs',
  'cjs',
  'ts',
  'tsx',
  'jsx',
  'php',
  'swift',
  'kt',
  'kts',
  'scala',
  'lua',
  'sql',
  'toml',
  'sh',
  'bash',
  'zsh',
  'dart',
  'html',
  'css',
  'scss',
  'less',
  'vue',
  'svelte',
  'gradle',
  'properties',
  'r',
  'pl',
];

export function useAttachments(deps: AttachmentsDeps) {
  const { setPendingFiles, setIsProcessing } = deps;

  /**
   * Store the picked file and stage it (no duplicates by stored id).
   * Hardening: the frontend pre-validates the extension whitelist to avoid pointless calls.
   * @param filePath - absolute path of the file
   * @param fileType - file type hint ('file' or 'image')
   */
  const handleFileUpload = async (filePath: string, fileType: 'file' | 'image') => {
    const fileName = filePath.split(/[\\/]/).pop() || t('chat.unknownFile');
    const ext = (fileName.split('.').pop() || '').toLowerCase();
    const isImg = fileType === 'image' || ALLOWED_IMG.includes(ext);
    const isDoc = ALLOWED_DOC.includes(ext);
    if (!isImg && !isDoc) {
      alert(
        t('chat.unsupportedFile', {
          extension: ext,
          supported: [...ALLOWED_IMG, ...ALLOWED_DOC].join(', '),
        }),
      );
      return;
    }
    setIsProcessing(true);
    try {
      const stored = await storeChatAttachment(filePath);
      setPendingFiles((prev) =>
        prev.some((file) => file.id === stored.id)
          ? prev
          : [
              ...prev,
              {
                ...stored,
                name: fileName,
                type: isImg ? 'image' : 'text',
                previewUrl: isImg ? convertFileSrc(stored.storagePath) : undefined,
              },
            ],
      );
    } catch (err) {
      alert(err);
    } finally {
      setIsProcessing(false);
    }
  };

  return { handleFileUpload };
}
