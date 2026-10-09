// Attachment domain Tauri command wrappers (mirrors commands/attachment.rs).
import { invoke } from '@tauri-apps/api/core';
import type { StoredAttachment } from '../store/types';

/** Register a local file as a chat attachment (copied into attachment storage with text extracted); returns attachment metadata. */
export const storeChatAttachment = (path: string) =>
  invoke<StoredAttachment>('store_chat_attachment', { path });

/** Discard an attachment not yet linked to a message (undo an unsent upload). */
export const discardChatAttachment = (attachmentId: string) =>
  invoke<void>('discard_chat_attachment', { attachmentId });

/** Save binary content to a file (export/download). */
export const saveBinaryFile = (path: string, bytes: number[]) =>
  invoke<void>('save_binary_file', { path, bytes });

/** Copy a generated/stored image to a target path (download). */
export const copyStoredImage = (src: string, dest: string) =>
  invoke<void>('copy_stored_image', { src, dest });
